import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useMemo, useRef, useState, type CSSProperties, type Dispatch, type SetStateAction } from "react";
import { ANIMAL_ART, ANIMAL_MOVEMENT_POLYGONS, ICON_ART, LAND_LAYOUT, PLOT_SLOTS, SCENE_ART, SPECIES, UI_TOKENS, compactNumber, formatCent, formatDuration, formatRatePerSecond, quantityLimit, type EconomySnapshot, type PlantSlotId, type Quantities, type ScenePolygon, type SpriteFrame, type TierSnapshot } from "./game";

type PermissionState = "unknown" | "allowed" | "denied" | "unavailable";
type InputHealth = "starting" | "healthy" | "degraded" | "stopped";
type Snapshot = { permission: PermissionState; inputPermissionRequired: boolean; inputHealth: InputHealth; totalEffectiveInputs: number; economy: EconomySnapshot };
type TradeQuote = { total: string; emergencyFree: boolean };
type Panel = "warehouse" | "buy" | "sell" | "catalog" | "stats" | "notice" | null;
type Point = { x: number; y: number };
type RoamController = { click: () => void; eat: () => void };
const emptyQuantities = (): Quantities => Object.fromEntries(SPECIES.flatMap((tier) => [[tier.id, 0], [tier.animalId, 0]]));

const [SCENE_WIDTH, SCENE_HEIGHT] = LAND_LAYOUT.coordinateSpace.canvas;
const [CROP_X, CROP_Y, CROP_WIDTH, CROP_HEIGHT] = LAND_LAYOUT.presentation.tightCropRect;
const FARM_STYLE: CSSProperties = { aspectRatio: `${CROP_WIDTH} / ${CROP_HEIGHT}` };
const SCENE_STAGE_STYLE: CSSProperties = {
  left: `${-CROP_X / CROP_WIDTH * 100}%`,
  top: `${-CROP_Y / CROP_HEIGHT * 100}%`,
  width: `${SCENE_WIDTH / CROP_WIDTH * 100}%`,
  height: `${SCENE_HEIGHT / CROP_HEIGHT * 100}%`,
};
const WINDOW_DRAG_STYLE = {
  ...sceneTupleRect(LAND_LAYOUT.windowDragControl.hitRect),
  "--window-drag-visual-width": `${LAND_LAYOUT.windowDragControl.visualSize[0]}px`,
  "--window-drag-visual-height": `${LAND_LAYOUT.windowDragControl.visualSize[1]}px`,
} as CSSProperties;
const WINDOW_HIDE_STYLE = {
  ...sceneTupleRect(LAND_LAYOUT.windowHideControl.hitRect),
  "--window-hide-visual-width": `${LAND_LAYOUT.windowHideControl.visualSize[0]}px`,
  "--window-hide-visual-height": `${LAND_LAYOUT.windowHideControl.visualSize[1]}px`,
} as CSSProperties;
const ROAM_STEP = PLOT_SLOTS.reduce((sum, plot) => sum + plot.w, 0) / PLOT_SLOTS.length * .28;
const ANNOUNCEMENT_LOGO = "/ui/branding/logo-clickclackfarm-masthead-final.png";
export function App() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [panel, setPanel] = useState<Panel>(null);
  const [plants, setPlants] = useState<Quantities>(emptyQuantities);
  const [animals, setAnimals] = useState<Quantities>(emptyQuantities);
  const [sales, setSales] = useState<Quantities>(emptyQuantities);
  const [quote, setQuote] = useState<TradeQuote | null>(null);
  const [tradeNotice, setTradeNotice] = useState("");
  const [, setMessage] = useState("欢迎来到敲敲牧场！继续使用电脑，就会帮助动物成长。");
  const [busy, setBusy] = useState(false);
  const [eatCycles, setEatCycles] = useState<Quantities>(emptyQuantities);
  const reducedMotion = useReducedMotion();
  const saveRecoveryReported = useRef(false);
  const previousEconomy = useRef<EconomySnapshot | null>(null);
  const permissionRequestInFlight = useRef(false);

  useEffect(() => {
    const tick = async () => {
      try {
        const next = await invoke<Snapshot>("game_snapshot");
        setSnapshot(next);
        const previous = previousEconomy.current;
        if (previous) {
          const inputDelta = BigInt(next.economy.lifetimeInputs) - BigInt(previous.lifetimeInputs);
          if (inputDelta > 0n) {
            const fedAnimals = SPECIES.filter((tier) => {
              const before = previous.tiers[tier.id];
              const after = next.economy.tiers[tier.id];
              const totalProgress = BigInt(before.feedingProgress) + inputDelta;
              return BigInt(before.animalCount) > 0n
                && BigInt(after.animalCount) > 0n
                && totalProgress >= 12n
                && Number(totalProgress % 12n) === after.feedingProgress;
            });
            if (fedAnimals.length > 0) setEatCycles((current) => {
              const updated = { ...current };
              for (const tier of fedAnimals) updated[tier.animalId] = (updated[tier.animalId] || 0) + 1;
              return updated;
            });
          }
        }
        previousEconomy.current = next.economy;
        if (next.economy.saveStatus === "recovered" && !saveRecoveryReported.current) {
          saveRecoveryReported.current = true;
          setMessage("旧存档无法读取，已安全开始新游戏。");
        }
      }
      catch (error) { setMessage(errorMessage(error, "暂时无法读取游戏状态。")); }
    };
    void tick();
    const timer = window.setInterval(() => void tick(), 500);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (!focused || permissionRequestInFlight.current) return;
      void invoke<boolean>("refresh_input_permission").then(async () => {
        const next = await invoke<Snapshot>("game_snapshot");
        if (!disposed) setSnapshot(next);
      }).catch(() => undefined);
    }).then((cleanup) => {
      if (disposed) cleanup(); else unlisten = cleanup;
    });
    return () => { disposed = true; unlisten?.(); };
  }, []);

  const hasBuySelection = useMemo(() => SPECIES.some((tier) => (plants[tier.id] || 0) > 0 || (animals[tier.animalId] || 0) > 0), [plants, animals]);
  const hasSaleSelection = useMemo(() => SPECIES.some((tier) => (sales[tier.animalId] || 0) > 0), [sales]);

  useEffect(() => {
    let current = true;
    setQuote(null);
    if (panel === "buy" && hasBuySelection) {
      void invoke<TradeQuote>("quote_purchase", { request: { plants, animals } }).then((value) => { if (current) setQuote(value); }).catch(() => undefined);
    } else if (panel === "sell" && hasSaleSelection) {
      void invoke<TradeQuote>("quote_sale", { request: { animals: sales } }).then((value) => { if (current) setQuote(value); }).catch(() => undefined);
    }
    return () => { current = false; };
  }, [panel, plants, animals, sales, hasBuySelection, hasSaleSelection]);

  const openPanel = (next: Exclude<Panel, null>) => {
    setPlants(emptyQuantities()); setAnimals(emptyQuantities()); setSales(emptyQuantities());
    setQuote(null); setTradeNotice(""); setPanel(next);
  };
  useEffect(() => {
    if (!tradeNotice) return;
    const timer = window.setTimeout(() => setTradeNotice(""), UI_TOKENS.motion.toastDuration);
    return () => window.clearTimeout(timer);
  }, [tradeNotice]);
  const requestPermission = async () => {
    setBusy(true);
    permissionRequestInFlight.current = true;
    try {
      const allowed = await invoke<boolean>("request_input_permission");
      const next = await invoke<Snapshot>("game_snapshot");
      setSnapshot(next);
      if (allowed) return;
      await invoke("open_input_monitoring_settings");
    } catch (error) {
      setMessage(errorMessage(error, "权限请求未完成，请稍后重试。"));
      void invoke("refresh_input_permission").catch(() => undefined);
    } finally {
      permissionRequestInFlight.current = false;
      setBusy(false);
    }
  };
  const startWindowDrag = (button: number) => {
    if (button === 0) void getCurrentWindow().startDragging();
  };
  const hideMainWindow = () => {
    void getCurrentWindow().hide();
  };
  const confirmTransaction = async () => {
    if (!panel || (panel !== "buy" && panel !== "sell") || !quote) return;
    const transactionPanel = panel;
    const transactionTotal = quote.total;
    const soldCount = Object.values(sales).reduce((sum, value) => sum + value, 0);
    setTradeNotice("");
    setBusy(true);
    try {
      const economy = panel === "buy"
        ? await invoke<EconomySnapshot>("commit_purchase", { request: { plants, animals } })
        : await invoke<EconomySnapshot>("commit_sale", { request: { animals: sales } });
      setSnapshot((current) => current ? { ...current, economy } : current);
      const successNotice = transactionPanel === "buy"
        ? `购买成功，花费 ${transactionTotal} 金币。`
        : `售出 ${soldCount} 只动物，获得 ${transactionTotal} 金币。`;
      setMessage(successNotice);
      setTradeNotice(successNotice);
      if (transactionPanel === "buy") { setPlants(emptyQuantities()); setAnimals(emptyQuantities()); }
      else setSales(emptyQuantities());
      setQuote(null);
    } catch (error) { setMessage(errorMessage(error, "交易失败")); }
    finally { setBusy(false); }
  };
  const game = snapshot?.economy;
  if (!game) return <main className="permission-screen"><section className="permission-card">正在载入牧场…</section></main>;

  return <main className="game-shell">
    <section className="farm" aria-label="牧场主画面" style={FARM_STYLE}>
      <div className="scene-stage" style={SCENE_STAGE_STYLE}>
        <img className="scene-base" src={SCENE_ART.base} alt="" draggable={false} style={{ transform: `scale(${LAND_LAYOUT.presentation.baseScale})` }} />
        {SPECIES.map((tier, index) => <FarmPlot key={tier.id} tier={tier} state={game.tiers[tier.id]} slot={PLOT_SLOTS[index]} />)}
        <img className="warehouse-art" src={SCENE_ART.warehouse} alt="" draggable={false} style={{ ...sceneTupleRect(LAND_LAYOUT.warehouse.renderRect), transform: `scale(${LAND_LAYOUT.warehouse.visualScale})` }} />
        <div className="actor-layer" aria-label="牧场中的动植物">
        {SPECIES.flatMap((tier, speciesIndex) => {
          const state = game.tiers[tier.id];
          const logicalCount = BigInt(state.plantCount);
          const visibleCount = Number(logicalCount > 5n ? 5n : logicalCount);
          if (visibleCount === 0) return [];
          const slotIds = LAND_LAYOUT.plantSlotPattern.visibleCount[String(visibleCount) as "1" | "2" | "3" | "4" | "5"];
          const plot = PLOT_SLOTS[speciesIndex];
          const feedingNeed = (BigInt(state.animalCount) > 0n ? BigInt(state.animalCount) : 1n) * tier.biteCent;
          const mature = BigInt(state.stockCent) >= feedingNeed;
          const art = mature ? SCENE_ART.plants[tier.id].full : SCENE_ART.plants[tier.id].seedling;
          const displayScale = mature && tier.id === "clover" ? UI_TOKENS.scenePlant.cloverDisplayScale : 1;
          return slotIds.map((slotId, index) => <PlantActor key={`${tier.id}-${slotId}`} name={tier.plant} art={art} point={plot.plantSlots[slotId]} mature={mature} displayScale={displayScale} announce={index === 0} />);
        })}
        {SPECIES.flatMap((tier, speciesIndex) => {
          const logicalCount = BigInt(game.tiers[tier.id].animalCount);
          const visibleCount = Number(logicalCount > 5n ? 5n : logicalCount);
          return Array.from({ length: visibleCount }, (_, instanceIndex) => <AnimalRoamer
            key={`${tier.animalId}-${instanceIndex}`}
            animalId={tier.animalId}
            name={tier.animal}
            instanceIndex={instanceIndex}
            initialPlot={PLOT_SLOTS[speciesIndex]}
            eatCycle={eatCycles[tier.animalId] || 0}
            reducedMotion={reducedMotion}
          />);
        })}
        </div>
        <button className="warehouse-button" style={polygonClipStyle(LAND_LAYOUT.warehouse.hitPolygon)} onClick={() => openPanel("warehouse")} aria-label="打开牧场管理" />
        <button className="window-drag" aria-label="拖动牧场窗口" style={WINDOW_DRAG_STYLE} onMouseDown={(event) => startWindowDrag(event.button)}><Icon name="windowDrag" /></button>
        <button className="window-hide" aria-label="隐藏牧场窗口（继续后台运行）" title="隐藏牧场（继续后台运行）" style={WINDOW_HIDE_STYLE} onClick={hideMainWindow}><Icon name="close" /></button>
      </div>
    </section>
    {panel && <TradePanel panel={panel} game={game} plants={plants} animals={animals} sales={sales} quote={quote} busy={busy}
      setPlants={setPlants} setAnimals={setAnimals} setSales={setSales} navigate={openPanel} close={() => { setTradeNotice(""); setPanel(null); }}
      tradeNotice={tradeNotice}
      confirmTransaction={confirmTransaction} />}
    {snapshot.permission !== "allowed" && <PermissionModal permission={snapshot.permission} permissionRequired={snapshot.inputPermissionRequired} busy={busy} requestPermission={requestPermission} />}
  </main>;
}

function FarmPlot({ tier, state, slot }: {
  tier: (typeof SPECIES)[number];
  state: TierSnapshot;
  slot: (typeof PLOT_SLOTS)[number];
}) {
  const plantCount = BigInt(state.plantCount);
  const cultivated = plantCount > 0n;

  return <article className="farm-plot" style={sceneRect(slot)} aria-label={`${tier.plant}与${tier.animal}`}>
    <img className="land-tile" src={cultivated ? SCENE_ART.landTiles.tilled : SCENE_ART.landTiles.grass} alt="" draggable={false} />
  </article>;
}

function PlantActor({ name, art, point, mature, displayScale, announce }: { name: string; art: string; point: readonly [number, number]; mature: boolean; displayScale: number; announce: boolean }) {
  const width = mature ? LAND_LAYOUT.plantRendering.matureWidthOfAuthoringCanvas : LAND_LAYOUT.plantRendering.seedlingWidthOfAuthoringCanvas;
  return <img className={`plant-actor ${mature ? "mature" : "seedling"}`} src={art} alt={announce ? name : ""} draggable={false} style={{ ...anchorStyle({ x: point[0], y: point[1] }), width: `${width * 100}%`, transform: `translate(-50%, -100%) scale(${displayScale})`, transformOrigin: UI_TOKENS.scenePlant.cloverTransformOrigin }} />;
}

function AnimalRoamer({ animalId, name, instanceIndex, initialPlot, eatCycle, reducedMotion }: {
  animalId: (typeof SPECIES)[number]["animalId"];
  name: string;
  instanceIndex: number;
  initialPlot: (typeof PLOT_SLOTS)[number];
  eatCycle: number;
  reducedMotion: boolean;
}) {
  const art = ANIMAL_ART[animalId];
  const initialPoint = useRef(randomPointInPolygon(initialPlot.animalFootSafePolygon));
  const positionRef = useRef(initialPoint.current);
  const [position, setPosition] = useState(initialPoint.current);
  const [sprite, setSprite] = useState({ action: art.idle, frame: art.idle.frames[0] });
  const [facingLeft, setFacingLeft] = useState(() => Math.random() < .5);
  const [movingVisual, setMovingVisual] = useState(false);
  const [reducedHop, setReducedHop] = useState(false);
  const controller = useRef<RoamController>({ click: () => undefined, eat: () => undefined });
  const previousEatCycle = useRef(eatCycle);

  useEffect(() => {
    const timers = new Set<number>();
    const frames = new Set<number>();
    let heading = Math.random() * 360;
    let queuedHeading: number | null = null;
    let segmentsLeft = 0;
    let moving = false;
    let eating = false;
    let target = positionRef.current;

    const schedule = (callback: () => void, delay: number) => {
      const timer = window.setTimeout(() => {
        timers.delete(timer);
        callback();
      }, delay);
      timers.add(timer);
      return timer;
    };
    const nextFrame = (callback: () => void) => {
      const frame = window.requestAnimationFrame(() => {
        frames.delete(frame);
        callback();
      });
      frames.add(frame);
    };
    const clearScheduled = () => {
      for (const timer of timers) window.clearTimeout(timer);
      for (const frame of frames) window.cancelAnimationFrame(frame);
      timers.clear();
      frames.clear();
    };
    const showIdle = () => setSprite({ action: art.idle, frame: art.idle.frames[0] });
    const setCurrentPosition = (point: Point) => {
      positionRef.current = point;
      setPosition(point);
    };

    const waitForRound = (first = false) => {
      moving = false;
      setMovingVisual(false);
      showIdle();
      if (document.hidden || eating) return;
      schedule(startRound, randomBetween(first ? 500 : 1000, first ? 2000 : 3000));
    };

    const finishSegment = () => {
      moving = false;
      setMovingVisual(false);
      setReducedHop(false);
      heading = queuedHeading ?? heading;
      const requestedHeading = queuedHeading;
      queuedHeading = null;
      segmentsLeft -= 1;
      if (segmentsLeft > 0 && !document.hidden && !eating) startSegment(requestedHeading);
      else waitForRound();
    };

    const animateWalk = (destination: Point) => {
      moving = true;
      setMovingVisual(!reducedMotion);
      if (reducedMotion) {
        setCurrentPosition(destination);
        setReducedHop(true);
        schedule(finishSegment, 180);
        return;
      }
      setSprite({ action: art.idle, frame: art.idle.frames[0] });
      for (let index = 1; index < art.idle.frames.length; index += 1) {
        schedule(() => setSprite({ action: art.idle, frame: art.idle.frames[index] }), index * 100);
      }
      nextFrame(() => setCurrentPosition(destination));
      schedule(finishSegment, 800);
    };

    function startSegment(requestedHeading: number | null = null) {
      if (document.hidden || eating) return;
      const move = chooseMove(positionRef.current, requestedHeading ?? heading);
      if (!move) {
        waitForRound();
        return;
      }
      heading = move.heading;
      target = move.point;
      const radians = heading * Math.PI / 180;
      const horizontal = Math.cos(radians);
      const vertical = Math.sin(radians);
      if (Math.abs(horizontal) >= Math.abs(vertical)) setFacingLeft(horizontal < 0);
      animateWalk(target);
    }

    function startRound() {
      if (document.hidden || eating) return;
      segmentsLeft = randomInteger(1, 3);
      startSegment();
    }

    const beginEat = () => {
      if (document.hidden) return;
      clearScheduled();
      moving = false;
      eating = true;
      queuedHeading = null;
      setMovingVisual(false);
      setReducedHop(false);
      let index = 0;
      const advance = () => {
        setSprite({ action: art.eat, frame: art.eat.frames[index] });
        schedule(() => {
          index += 1;
          if (index < art.eat.frames.length) advance();
          else {
            eating = false;
            showIdle();
            waitForRound(true);
          }
        }, art.eat.durationsMs[index]);
      };
      advance();
    };

    controller.current = {
      click: () => {
        if (document.hidden || eating) return;
        if (moving) {
          queuedHeading = chooseMove(target, heading)?.heading ?? null;
          return;
        }
        clearScheduled();
        segmentsLeft = 1;
        startSegment();
      },
      eat: beginEat,
    };

    const onVisibilityChange = () => {
      clearScheduled();
      moving = false;
      eating = false;
      queuedHeading = null;
      setMovingVisual(false);
      setReducedHop(false);
      showIdle();
      if (!document.hidden) waitForRound(true);
    };
    document.addEventListener("visibilitychange", onVisibilityChange);
    if (!document.hidden) waitForRound(true);
    return () => {
      clearScheduled();
      document.removeEventListener("visibilitychange", onVisibilityChange);
      controller.current = { click: () => undefined, eat: () => undefined };
    };
  }, [art, reducedMotion]);

  useEffect(() => {
    if (previousEatCycle.current === eatCycle) return;
    previousEatCycle.current = eatCycle;
    controller.current.eat();
  }, [eatCycle]);

  const roamerStyle: CSSProperties = {
    left: `${position.x / SCENE_WIDTH * 100}%`,
    top: `${position.y / SCENE_HEIGHT * 100}%`,
    transitionDuration: movingVisual ? "800ms" : "0ms",
    zIndex: Math.round(position.y),
  };
  const frameStyle: CSSProperties = {
    ...spriteStyle(sprite.action, sprite.frame),
    transform: `scaleX(${facingLeft ? -1 : 1})`,
  };

  return <button
    type="button"
    className={`animal-roamer animal-${animalId}${reducedHop ? " reduced-hop" : ""}`}
    aria-label={`${name} ${instanceIndex + 1}`}
    style={roamerStyle}
    onClick={() => controller.current.click()}
  ><span className="animal-frame" style={frameStyle} /></button>;
}

function useReducedMotion(): boolean {
  const [reduced, setReduced] = useState(() => window.matchMedia("(prefers-reduced-motion: reduce)").matches);
  useEffect(() => {
    const media = window.matchMedia("(prefers-reduced-motion: reduce)");
    const update = () => setReduced(media.matches);
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, []);
  return reduced;
}

function randomPointInPolygon(polygon: ScenePolygon): Point {
  const xs = polygon.map(([x]) => x);
  const ys = polygon.map(([, y]) => y);
  const bounds = { left: Math.min(...xs), right: Math.max(...xs), top: Math.min(...ys), bottom: Math.max(...ys) };
  for (let attempt = 0; attempt < 40; attempt += 1) {
    const point = { x: randomBetween(bounds.left, bounds.right), y: randomBetween(bounds.top, bounds.bottom) };
    if (pointInPolygon(point, polygon)) return point;
  }
  return { x: xs.reduce((sum, value) => sum + value, 0) / xs.length, y: ys.reduce((sum, value) => sum + value, 0) / ys.length };
}

function chooseMove(origin: Point, previousHeading: number): { point: Point; heading: number } | null {
  for (let attempt = 0; attempt < 12; attempt += 1) {
    const preferred = attempt < 6;
    const turn = preferred
      ? randomBetween(-60, 60)
      : randomBetween(90, 180) * (Math.random() < .5 ? -1 : 1);
    const heading = normalizeDegrees(previousHeading + turn);
    const radians = heading * Math.PI / 180;
    const point = { x: origin.x + Math.cos(radians) * ROAM_STEP, y: origin.y + Math.sin(radians) * ROAM_STEP };
    if (isAnimalPathAllowed(origin, point)) return { point, heading };
    if (attempt === 0) {
      const reflectedHeading = normalizeDegrees(previousHeading + 180);
      const reflectedRadians = reflectedHeading * Math.PI / 180;
      const reflectedPoint = { x: origin.x + Math.cos(reflectedRadians) * ROAM_STEP, y: origin.y + Math.sin(reflectedRadians) * ROAM_STEP };
      if (isAnimalPathAllowed(origin, reflectedPoint)) return { point: reflectedPoint, heading: reflectedHeading };
    }
  }
  return null;
}

function isAnimalPathAllowed(origin: Point, destination: Point): boolean {
  for (let sample = 1; sample <= 10; sample += 1) {
    const ratio = sample / 10;
    const point = { x: origin.x + (destination.x - origin.x) * ratio, y: origin.y + (destination.y - origin.y) * ratio };
    if (!ANIMAL_MOVEMENT_POLYGONS.some((polygon) => pointInPolygon(point, polygon)) || pointInPolygon(point, LAND_LAYOUT.warehouse.animalForbiddenPolygon)) return false;
  }
  return true;
}

function pointInPolygon(point: Point, polygon: ScenePolygon): boolean {
  let inside = false;
  for (let index = 0, previous = polygon.length - 1; index < polygon.length; previous = index, index += 1) {
    const [x1, y1] = polygon[previous];
    const [x2, y2] = polygon[index];
    const cross = (point.x - x1) * (y2 - y1) - (point.y - y1) * (x2 - x1);
    const onEdge = Math.abs(cross) < .001 && point.x >= Math.min(x1, x2) && point.x <= Math.max(x1, x2) && point.y >= Math.min(y1, y2) && point.y <= Math.max(y1, y2);
    if (onEdge) return true;
    if ((y1 > point.y) !== (y2 > point.y) && point.x < (x2 - x1) * (point.y - y1) / (y2 - y1) + x1) inside = !inside;
  }
  return inside;
}

function normalizeDegrees(value: number): number {
  return (value % 360 + 360) % 360;
}

function randomBetween(minimum: number, maximum: number): number {
  return minimum + Math.random() * (maximum - minimum);
}

function randomInteger(minimum: number, maximum: number): number {
  return Math.floor(randomBetween(minimum, maximum + 1));
}

function sceneRect(slot: (typeof PLOT_SLOTS)[number]): CSSProperties {
  return {
    left: `${slot.x / SCENE_WIDTH * 100}%`,
    top: `${slot.y / SCENE_HEIGHT * 100}%`,
    width: `${slot.w / SCENE_WIDTH * 100}%`,
    height: `${slot.h / SCENE_HEIGHT * 100}%`,
  };
}

function sceneTupleRect([x, y, w, h]: readonly [number, number, number, number]): CSSProperties {
  return { left: `${x / SCENE_WIDTH * 100}%`, top: `${y / SCENE_HEIGHT * 100}%`, width: `${w / SCENE_WIDTH * 100}%`, height: `${h / SCENE_HEIGHT * 100}%` };
}

function anchorStyle(point: Point): CSSProperties {
  return { left: `${point.x / SCENE_WIDTH * 100}%`, top: `${point.y / SCENE_HEIGHT * 100}%`, zIndex: Math.round(point.y) };
}

function polygonClipStyle(polygon: ScenePolygon): CSSProperties {
  const points = polygon.map(([x, y]) => `${x / SCENE_WIDTH * 100}% ${y / SCENE_HEIGHT * 100}%`).join(", ");
  return { clipPath: `polygon(${points})` };
}

function spriteStyle(action: { sheet: string; sheetWidth: number; sheetHeight: number }, spriteFrame: SpriteFrame): CSSProperties {
  const x = action.sheetWidth === spriteFrame.w ? 0 : spriteFrame.x / (action.sheetWidth - spriteFrame.w) * 100;
  const y = action.sheetHeight === spriteFrame.h ? 0 : spriteFrame.y / (action.sheetHeight - spriteFrame.h) * 100;
  return {
    backgroundImage: `url(${action.sheet})`,
    backgroundSize: `${action.sheetWidth / spriteFrame.w * 100}% ${action.sheetHeight / spriteFrame.h * 100}%`,
    backgroundPosition: `${x}% ${y}%`,
  };
}

function PermissionModal({ permission, permissionRequired, busy, requestPermission }: { permission: PermissionState; permissionRequired: boolean; busy: boolean; requestPermission: () => Promise<void> }) {
  if (!permissionRequired || permission === "unavailable") {
    return <div className="permission-overlay" role="presentation"><section className="permission-modal" role="alertdialog" aria-modal="true" aria-labelledby="permission-title" aria-describedby="permission-description">
      <h2 id="permission-title">输入计数不可用</h2>
      <p id="permission-description">敲敲牧场未能启动全局输入计数。请退出后重新启动游戏；若仍然出现此状态，请确认安全软件未拦截应用。</p>
    </section></div>;
  }
  return <div className="permission-overlay" role="presentation"><section className="permission-modal" role="dialog" aria-modal="true" aria-labelledby="permission-title" aria-describedby="permission-description">
    <h2 id="permission-title">开启“输入监控”权限</h2>
    <p id="permission-description">敲敲牧场需要“输入监控”权限，仅用于统计键盘和鼠标点击次数，游戏不会读取或保存输入内容。</p>
    <button className="primary large" disabled={busy} autoFocus onClick={() => void requestPermission()}>{busy ? "正在请求…" : "开启权限"}</button>
  </section></div>;
}

const PANEL_LABELS: Record<Exclude<Panel, null>, string> = {
  warehouse: "仓库",
  buy: "购买",
  sell: "出售",
  catalog: "图鉴",
  stats: "统计",
  notice: "公告",
};
const MANAGER_TOKEN_STYLE = buildManagerTokenStyle();

function TradePanel({ panel, game, plants, animals, sales, quote, busy, setPlants, setAnimals, setSales, navigate, close, tradeNotice, confirmTransaction }: {
  panel: Exclude<Panel, null>;
  game: EconomySnapshot;
  plants: Quantities;
  animals: Quantities;
  sales: Quantities;
  quote: TradeQuote | null;
  busy: boolean;
  setPlants: Dispatch<SetStateAction<Quantities>>;
  setAnimals: Dispatch<SetStateAction<Quantities>>;
  setSales: Dispatch<SetStateAction<Quantities>>;
  navigate: (panel: Exclude<Panel, null>) => void;
  close: () => void;
  tradeNotice: string;
  confirmTransaction: () => Promise<void>;
}) {
  const title = useRef<HTMLHeadingElement>(null);
  useEffect(() => { title.current?.focus(); }, [panel]);
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => { if (event.key === "Escape" && !busy) close(); };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [busy, close]);

  return <div className="overlay" style={MANAGER_TOKEN_STYLE} onMouseDown={(event) => { if (event.target === event.currentTarget) close(); }}>
    <section className="manager" role="dialog" aria-modal="true">
      <div className="panel-title">
        <h2 ref={title} tabIndex={-1}>敲敲仓库</h2>
        <button className="icon-button" aria-label="关闭" onClick={close}><Icon name="close" /></button>
      </div>
      <nav className="manager-tabs" aria-label="仓库功能">
        {(Object.keys(PANEL_LABELS) as Exclude<Panel, null>[]).map((tab) => <button className={panel === tab ? "active" : ""} aria-current={panel === tab ? "page" : undefined} onClick={() => navigate(tab)} key={tab}><Icon name={tab} /><span>{PANEL_LABELS[tab]}</span></button>)}
      </nav>
      {panel === "warehouse" && <WarehousePage game={game} />}
      {panel === "buy" && <BuyPage game={game} plants={plants} animals={animals} quote={quote} busy={busy} setPlants={setPlants} setAnimals={setAnimals} confirmTransaction={confirmTransaction} />}
      {panel === "sell" && <SellPage game={game} sales={sales} quote={quote} busy={busy} setSales={setSales} navigate={navigate} confirmTransaction={confirmTransaction} />}
      {panel === "catalog" && <CatalogPage game={game} />}
      {panel === "stats" && <StatsPage game={game} />}
      {panel === "notice" && <NoticePage />}
      {tradeNotice && <p className="trade-notice" role="status" aria-live="polite">{tradeNotice}</p>}
    </section>
  </div>;
}

function WarehousePage({ game }: { game: EconomySnapshot }) {
  return <div className="manager-page management-page-with-footer">
    <div className="manager-columns">
      <section><h3><Icon name="plant" />植物</h3><div className="compact-table warehouse-table"><div className="table-head"><span>名称</span><span>数量</span><span>{UI_TOKENS.copy.warehousePlantEfficiencyLabel}</span><span>已储存养分</span></div>{SPECIES.map((tier) => { const state = game.tiers[tier.id]; return <div className="table-line" key={tier.id}><PlantName tier={tier} /><span>{compactNumber(state.plantCount)}</span><span>{formatNutrientRate(state.totalRateCentPerMinute, UI_TOKENS.copy.warehousePlantEfficiencyValuePattern)}</span><span>{formatStoredNutrient(state.stockCent)}</span></div>; })}</div></section>
      <section><h3><Icon name="animal" />动物</h3><div className="compact-table warehouse-table"><div className="table-head"><span>名称</span><span>数量</span><span>单只价值</span><span>{UI_TOKENS.copy.warehouseAnimalConsumptionLabel}</span></div>{SPECIES.map((tier) => { const state = game.tiers[tier.id]; return <div className="table-line" key={tier.animalId}><AnimalName tier={tier} /><span>{compactNumber(state.animalCount)}</span><span>{state.singleSaleValueCoins === null ? "未拥有" : `${compactNumber(state.singleSaleValueCoins)} 金币`}</span><span>{formatWholeCent(state.groupBiteCent)}</span></div>; })}</div></section>
    </div>
    <div className="panel-footer"><strong className="footer-cash">{formatCopy(UI_TOKENS.transactionFooter.cashLabelPattern, { amount: formatFullInteger(game.coins) })}</strong></div>
  </div>;
}

function BuyPage({ game, plants, animals, quote, busy, setPlants, setAnimals, confirmTransaction }: {
  game: EconomySnapshot;
  plants: Quantities;
  animals: Quantities;
  quote: TradeQuote | null;
  busy: boolean;
  setPlants: Dispatch<SetStateAction<Quantities>>;
  setAnimals: Dispatch<SetStateAction<Quantities>>;
  confirmTransaction: () => Promise<void>;
}) {
  return <div className="manager-page management-page-with-footer">
    <div className="manager-columns">
      <section><h3><Icon name="plant" />购买种子</h3><div className="compact-table buy-table"><div className="table-head"><span>名称</span><span>售卖价格</span><span>当前数量</span><span>购买数量</span></div>{SPECIES.map((tier) => { const state = game.tiers[tier.id]; return <div className="table-line" key={tier.id}><PlantName tier={tier} /><span>{compactNumber(state.nextSeedPriceCoins)} 金币</span><span>{compactNumber(state.plantCount)}</span><BuyQuantity value={plants[tier.id] || 0} setValue={(value) => setPlants((current) => ({ ...current, [tier.id]: value }))} label={`${tier.plant}购买数量`} /></div>; })}</div></section>
      <section><h3><Icon name="animal" />购买动物</h3><div className="compact-table buy-table"><div className="table-head"><span>名称</span><span>售卖价格</span><span>当前数量</span><span>购买数量</span></div>{SPECIES.map((tier) => { const state = game.tiers[tier.id]; return <div className="table-line" key={tier.animalId}><AnimalName tier={tier} /><span>{compactNumber(state.animalPurchasePriceCoins)} 金币</span><span>{compactNumber(state.animalCount)}</span><BuyQuantity value={animals[tier.animalId] || 0} setValue={(value) => setAnimals((current) => ({ ...current, [tier.animalId]: value }))} max={tier.animalId === "rabbit" && quote?.emergencyFree ? 1 : 999} label={`${tier.animal}购买数量`} /></div>; })}</div></section>
    </div>
    <div className="panel-footer"><strong className="footer-cash">{formatCopy(UI_TOKENS.transactionFooter.cashLabelPattern, { amount: formatFullInteger(game.coins) })}</strong><div className="footer-transaction"><strong className="footer-summary">{formatCopy(UI_TOKENS.copy.purchaseTotalPattern, { amount: quote?.total ?? "0" })}</strong><button className="primary" disabled={!quote || busy} onClick={() => void confirmTransaction()}>确认购买</button></div></div>
  </div>;
}

function SellPage({ game, sales, quote, busy, setSales, navigate, confirmTransaction }: {
  game: EconomySnapshot;
  sales: Quantities;
  quote: TradeQuote | null;
  busy: boolean;
  setSales: Dispatch<SetStateAction<Quantities>>;
  navigate: (panel: Exclude<Panel, null>) => void;
  confirmTransaction: () => Promise<void>;
}) {
  const owned = SPECIES.filter((tier) => BigInt(game.tiers[tier.id].animalCount) > 0n);
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const updateValue = (animalId: string, value: number) => {
    setDrafts((current) => ({ ...current, [animalId]: String(value) }));
    setSales((current) => ({ ...current, [animalId]: value }));
  };
  const updateDraft = (animalId: string, value: string, max: number) => {
    if (/^\d+$/.test(value) && BigInt(value) > BigInt(max)) {
      updateValue(animalId, max);
      return;
    }
    setDrafts((current) => ({ ...current, [animalId]: value }));
    const parsed = parseSellQuantity(value, max);
    if (parsed !== null) setSales((current) => ({ ...current, [animalId]: parsed }));
  };
  const invalid = owned.some((tier) => {
    const max = quantityLimit(game.tiers[tier.id].animalCount);
    return parseSellQuantity(drafts[tier.animalId] ?? String(sales[tier.animalId] || 0), max) === null;
  });
  return <div className="manager-page management-page-with-footer">
    <section className="sell-section"><h3><Icon name="animal" />出售动物</h3>
      {owned.length === 0 ? <div className="empty-page"><button className="primary" onClick={() => navigate("buy")}>购买</button></div> : <div className="compact-table sell-table"><div className="table-head"><span>名称</span><span>单只价值</span><span>{UI_TOKENS.copy.sellOwnedQuantityLabel}</span><span>出售数量</span></div>{owned.map((tier) => { const max = quantityLimit(game.tiers[tier.id].animalCount); const value = sales[tier.animalId] || 0; const draft = drafts[tier.animalId] ?? String(value); return <div className="table-line" key={tier.animalId}><AnimalName tier={tier} /><span>{compactNumber(game.tiers[tier.id].singleSaleValueCoins || "0")} 金币</span><span>{compactNumber(game.tiers[tier.id].animalCount)}</span><SellQuantity value={value} draft={draft} max={max} setValue={(next) => updateValue(tier.animalId, next)} setDraft={(next) => updateDraft(tier.animalId, next, max)} onBlur={() => { if (draft === "") updateValue(tier.animalId, 0); }} label={`${tier.animal}出售数量`} /></div>; })}</div>}
    </section>
    <div className="panel-footer"><strong className="footer-cash">{formatCopy(UI_TOKENS.transactionFooter.cashLabelPattern, { amount: formatFullInteger(game.coins) })}</strong><div className="footer-transaction"><strong className="footer-summary">{formatCopy(UI_TOKENS.copy.saleTotalPattern, { amount: quote?.total ?? "0" })}</strong><button className="primary" disabled={!quote || busy || invalid} onClick={() => void confirmTransaction()}>确认售出</button></div></div>
  </div>;
}

function CatalogPage({ game }: { game: EconomySnapshot }) {
  const [kind, setKind] = useState<"plant" | "animal">("plant");
  const [index, setIndex] = useState(0);
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "ArrowLeft") setIndex((value) => Math.max(0, value - 1));
      if (event.key === "ArrowRight") setIndex((value) => Math.min(SPECIES.length - 1, value + 1));
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
  const tier = SPECIES[index];
  const count = kind === "plant" ? game.tiers[tier.id].plantCount : game.tiers[tier.id].animalCount;
  const lit = kind === "plant" ? tierState(game, tier).plantDiscovered : tierState(game, tier).animalDiscovered;
  const selectKind = (next: "plant" | "animal") => { setKind(next); setIndex(0); };
  return <div className="manager-page catalog-page">
    <div className="catalog-tabs"><button className={kind === "plant" ? "active" : ""} onClick={() => selectKind("plant")}><Icon name="plant" />植物图鉴</button><button className={kind === "animal" ? "active" : ""} onClick={() => selectKind("animal")}><Icon name="animal" />动物图鉴</button></div>
    <div className="catalog-nav">{index === 0 ? <span className="pager-placeholder" aria-hidden="true" /> : <button className="pager-button" aria-label="上一页" onClick={() => setIndex((value) => value - 1)}><Icon name="previous" /></button>}<article className="catalog-content">
      <div className={`catalog-art ${kind}${lit ? "" : " unlit"}`} role="img" aria-label={`${kind === "plant" ? tier.plant : tier.animal}，${lit ? "已点亮" : "未点亮"}`}>{kind === "plant" ? <img src={SCENE_ART.plants[tier.id].compendium} alt="" /> : <img src={ANIMAL_ART[tier.animalId].portrait} alt="" />}</div>
      <div className="catalog-copy"><h3>{kind === "plant" ? tier.plant : tier.animal}</h3>
        {!lit && <span className="catalog-status">未点亮</span>}
        {kind === "plant" ? <dl><div><dt>当前种子价格</dt><dd>{compactNumber(tierState(game, tier).nextSeedPriceCoins)} 金币</dd></div><div><dt>{UI_TOKENS.copy.compendiumPlantEfficiencyLabel}</dt><dd>{formatNutrientRate(tierState(game, tier).plantRateCentPerMinute, UI_TOKENS.copy.compendiumPlantEfficiencyValuePattern)}</dd></div><div><dt>{UI_TOKENS.copy.compendiumPlantOwnedQuantityLabel}</dt><dd>{compactNumber(count)}</dd></div></dl> : <dl><div><dt>{UI_TOKENS.copy.compendiumAnimalGrowthLabel}</dt><dd>{formatCent(tierState(game, tier).growthPerFeedCent)}</dd></div><div><dt>当前数量</dt><dd>{compactNumber(count)}</dd></div><div><dt>{UI_TOKENS.copy.compendiumAnimalLifetimePurchasedLabel}</dt><dd>{compactNumber(tierState(game, tier).lifetimePaidPurchaseCount)}</dd></div></dl>}
      </div>
    </article>{index === SPECIES.length - 1 ? <span className="pager-placeholder" aria-hidden="true" /> : <button className="pager-button" aria-label="下一页" onClick={() => setIndex((value) => value + 1)}><Icon name="next" /></button>}</div>
  </div>;
}

function StatsPage({ game }: { game: EconomySnapshot }) {
  const statistics = game.statistics;
  return <div className="manager-page stats-page"><section><h3>今天</h3><dl><div><dt>屏幕点亮时间</dt><dd>{formatDuration(statistics.todayProductiveSeconds)}</dd></div><div><dt>{UI_TOKENS.copy.statisticsInputCountLabel}</dt><dd>{formatInputCount(statistics.todayInputs)}</dd></div></dl></section><section><h3>总计</h3><dl><div><dt>{UI_TOKENS.copy.statisticsLifetimeDaysLabel}</dt><dd>{formatPlainInteger(statistics.productiveDays)} 天</dd></div><div><dt>屏幕点亮时间</dt><dd>{formatDuration(statistics.lifetimeProductiveSeconds)}</dd></div><div><dt>{UI_TOKENS.copy.statisticsInputCountLabel}</dt><dd>{formatInputCount(statistics.lifetimeInputs)}</dd></div></dl></section></div>;
}

function NoticePage() {
  return <div className="manager-page notice-page"><article className="notice-content">
    <img className="notice-logo" src={ANNOUNCEMENT_LOGO} alt="敲敲牧场，小白兔和小仓鼠" />
    <div className="notice-copy">
      <p className="notice-emphasis">{UI_TOKENS.copy.announcementTitle}</p>
      <p>{UI_TOKENS.copy.announcementIntro}</p>
      <p>{UI_TOKENS.copy.announcementPlantLine}</p>
      <p>{UI_TOKENS.copy.announcementAnimalLine}</p>
      <p>{UI_TOKENS.copy.announcementBackgroundLine}</p>
      <p className="notice-emphasis">{UI_TOKENS.copy.announcementSlogan}</p>
      <p className="notice-contact">如果你在游戏中遇到问题或者有其他的建议，请通过邮件联系我：<a href={UI_TOKENS.announcement.emailHref}>{UI_TOKENS.announcement.email}</a>，感谢你的反馈！</p>
      <p className="notice-creator">{UI_TOKENS.copy.announcementCreator}</p>
    </div>
  </article></div>;
}

function PlantName({ tier }: { tier: (typeof SPECIES)[number] }) {
  return <span className="trade-portrait"><span className={`portrait-frame plant plant-${tier.id}`}><img src={SCENE_ART.plants[tier.id].full} alt="" /></span>{tier.plant}</span>;
}

function AnimalName({ tier }: { tier: (typeof SPECIES)[number] }) {
  return <span className="trade-portrait"><span className="portrait-frame animal"><img src={ANIMAL_ART[tier.animalId].portrait} alt="" /></span>{tier.animal}</span>;
}

function tierState(game: EconomySnapshot, tier: (typeof SPECIES)[number]) {
  return game.tiers[tier.id];
}

function formatStoredNutrient(stockCent: string): string {
  const divisor = BigInt(UI_TOKENS.numberDisplay.storedNutrientDivisor);
  return (BigInt(stockCent || "0") / divisor).toLocaleString("zh-CN");
}

function formatFullInteger(value: string): string {
  return BigInt(value || "0").toLocaleString("zh-CN");
}

function formatPlainInteger(value: string): string {
  return BigInt(value || "0").toString(10);
}

function formatInputCount(value: string): string {
  return formatCopy(UI_TOKENS.copy.statisticsInputCountValuePattern, { value: formatPlainInteger(value) });
}

function formatCopy(pattern: string, values: Record<string, string>): string {
  return pattern.replace(/\{(\w+)\}/g, (_, name: string) => values[name] ?? "");
}

function formatNutrientRate(rateCentPerMinute: string, pattern: string): string {
  return formatCopy(pattern, { value: formatRatePerSecond(rateCentPerMinute).replace(/\/s$/, "") });
}

function formatWholeCent(value: string): string {
  return (BigInt(value || "0") / 100n).toLocaleString("zh-CN");
}

function BuyQuantity({ value, setValue, label, max = 999 }: { value: number; setValue: (value: number) => void; label: string; max?: number }) {
  return <span className="stepper"><button aria-label={`${label}减少`} disabled={value === 0} onClick={() => setValue(value - 1)}><Icon name="minus" /></button><input aria-label={label} inputMode="numeric" value={value} onChange={(event) => setValue(Math.max(0, Math.min(max, Math.floor(Number(event.target.value) || 0))))} /><button aria-label={`${label}增加`} disabled={value >= max} onClick={() => setValue(value + 1)}><Icon name="plus" /></button></span>;
}

function parseSellQuantity(value: string, max: number): number | null {
  if (!/^\d+$/.test(value)) return null;
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) && parsed >= 0 && parsed <= max ? parsed : null;
}

function SellQuantity({ value, draft, setValue, setDraft, onBlur, max, label }: { value: number; draft: string; setValue: (value: number) => void; setDraft: (value: string) => void; onBlur: () => void; max: number; label: string }) {
  return <span className="stepper sell-stepper"><button aria-label={`${label}减少`} disabled={value === 0} onClick={() => setValue(Math.max(0, value - 1))}><Icon name="minus" /></button><input aria-label={label} inputMode="numeric" value={draft} aria-invalid={parseSellQuantity(draft, max) === null} onChange={(event) => setDraft(event.target.value)} onBlur={onBlur} /><button aria-label={`${label}增加`} disabled={value >= max} onClick={() => setValue(Math.min(max, value + 1))}><Icon name="plus" /></button></span>;
}

function Icon({ name }: { name: keyof typeof ICON_ART }) {
  const source = `url(${ICON_ART[name]})`;
  return <span className="ui-icon" aria-hidden="true" style={{ WebkitMaskImage: source, maskImage: source }} />;
}

function buildManagerTokenStyle(): CSSProperties {
  const variables: Record<string, string> = {};
  const kebab = (value: string) => value.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`);
  for (const [name, value] of Object.entries(UI_TOKENS.color)) variables[`--cc-color-${kebab(name)}`] = String(value);
  for (const [name, value] of Object.entries(UI_TOKENS.spacing)) variables[`--cc-space-${name}`] = `${value}px`;
  for (const [name, value] of Object.entries(UI_TOKENS.radius)) variables[`--cc-radius-${kebab(name)}`] = `${value}px`;
  for (const [name, value] of Object.entries(UI_TOKENS.stroke)) variables[`--cc-stroke-${kebab(name)}`] = `${value}px`;
  for (const [name, value] of Object.entries(UI_TOKENS.control)) variables[`--cc-control-${kebab(name)}`] = `${value}px`;
  for (const [name, value] of Object.entries(UI_TOKENS.layout)) variables[`--cc-layout-${kebab(name)}`] = typeof value === "number" ? `${value}px` : String(value);
  for (const [name, value] of Object.entries(UI_TOKENS.motion)) variables[`--cc-motion-${kebab(name)}`] = `${value}ms`;
  variables["--cc-font-family"] = UI_TOKENS.typography.fontFamily.map((name) => name.includes(" ") ? `"${name}"` : name).join(", ");
  for (const [styleName, style] of Object.entries(UI_TOKENS.typography.styles)) {
    variables[`--cc-font-${kebab(styleName)}-size`] = `${style.size}px`;
    variables[`--cc-font-${kebab(styleName)}-line-height`] = `${style.lineHeight}px`;
    variables[`--cc-font-${kebab(styleName)}-weight`] = String(style.weight);
  }
  for (const [name, value] of Object.entries(UI_TOKENS.icon)) variables[`--cc-icon-${kebab(name)}`] = typeof value === "number" && name !== "strokeWidth" ? `${value}px` : String(value);
  for (const [name, value] of Object.entries(UI_TOKENS.portrait)) {
    const unit = typeof value === "number" && (name.endsWith("Container") || name.endsWith("Padding") || name === "radius") ? "px" : "";
    variables[`--cc-portrait-${kebab(name)}`] = `${value}${unit}`;
  }
  for (const [name, value] of Object.entries(UI_TOKENS.sellTable)) variables[`--cc-sell-table-${kebab(name)}`] = typeof value === "number" ? `${value}px` : String(value);
  for (const [name, value] of Object.entries(UI_TOKENS.transactionFooter)) variables[`--cc-transaction-footer-${kebab(name)}`] = typeof value === "number" ? `${value}px` : String(value);
  for (const [name, value] of Object.entries(UI_TOKENS.compendium)) variables[`--cc-compendium-${kebab(name)}`] = typeof value === "number" ? `${value}px` : String(value);
  variables["--cc-announcement-content-max-width"] = `${UI_TOKENS.announcement.contentMaxWidth}px`;
  variables["--cc-announcement-logo-width"] = UI_TOKENS.announcement.logoWidthCss;
  variables["--cc-announcement-logo-to-copy-gap"] = `${UI_TOKENS.announcement.logoToCopyGap}px`;
  variables["--cc-announcement-copy-line-gap"] = `${UI_TOKENS.announcement.copyLineGap}px`;
  variables["--cc-announcement-contact-gap"] = `${UI_TOKENS.announcement.contactGap}px`;
  variables["--cc-compendium-plant-image-safe-area"] = `${UI_TOKENS.compendium.plantImageSafeArea * 100}%`;
  variables["--cc-compendium-plant-image-content-limit"] = `${(1 - (2 * UI_TOKENS.compendium.plantImageSafeArea)) * 100}%`;
  variables["--cc-font-page-title"] = variables["--cc-font-page-title-size"];
  variables["--cc-font-section-title"] = variables["--cc-font-section-title-size"];
  variables["--cc-font-body"] = variables["--cc-font-body-size"];
  variables["--cc-font-label"] = variables["--cc-font-label-size"];
  variables["--cc-font-caption"] = variables["--cc-font-caption-size"];
  variables["--cc-icon-small"] = variables["--cc-icon-size-small"];
  variables["--cc-icon-default"] = variables["--cc-icon-size-default"];
  variables["--cc-icon-navigation"] = variables["--cc-icon-size-navigation"];
  variables["--cc-portrait-list"] = variables["--cc-portrait-list-container"];
  variables["--cc-portrait-detail"] = variables["--cc-portrait-detail-container"];
  return variables as CSSProperties;
}

function errorMessage(error: unknown, fallback: string) { return typeof error === "string" && error ? error : fallback; }
