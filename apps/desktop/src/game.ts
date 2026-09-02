import landLayoutJson from "./land-layout.json";
import uiTokensJson from "./ui-tokens.json";

/** Frontend-only display metadata. All game rules and economic values live in Rust. */
export const SPECIES = [
  { id: "clover", plant: "三叶草", animalId: "rabbit", animal: "小白兔", biteCent: 200n },
  { id: "sunflower", plant: "向日葵", animalId: "hamster", animal: "小仓鼠", biteCent: 1_000n },
  { id: "bamboo", plant: "竹子", animalId: "red-panda", animal: "小熊猫", biteCent: 5_000n },
  { id: "corn", plant: "玉米", animalId: "capybara", animal: "卡皮巴拉", biteCent: 25_000n },
  { id: "apple", plant: "苹果树", animalId: "deer", animal: "小鹿", biteCent: 125_000n },
] as const;

export type SpriteFrame = { x: number; y: number; w: number; h: number };
type SpriteAction = {
  sheet: string;
  sheetWidth: number;
  sheetHeight: number;
  frames: readonly SpriteFrame[];
  durationsMs: readonly number[];
};
type AnimalArt = {
  portrait: string;
  idle: Omit<SpriteAction, "durationsMs">;
  eat: SpriteAction;
};

const ART_ROOT = "/art/v0.1.1";
const frame = (x: number, y: number): SpriteFrame => ({ x, y, w: 256, h: 256 });
const layout = (y: number, xs: readonly number[]): readonly SpriteFrame[] => xs.map((x) => frame(x, y));
const deliveredRow = [0, 256, 512, 768, 1024, 1280, 1536, 1792] as const;

/**
 * Runtime sprite coordinates transcribed from the v0.1.1 animal manifests.
 * Keep the explicit action arrays: some delivered sequences intentionally reuse
 * cells (notably the capybara eat action) instead of moving across a full row.
 */
export const ANIMAL_ART: Record<(typeof SPECIES)[number]["animalId"], AnimalArt> = {
  rabbit: {
    portrait: `${ART_ROOT}/portraits/rabbit.png`,
    idle: { sheet: `${ART_ROOT}/animals/rabbit/move.png`, sheetWidth: 2048, sheetHeight: 256, frames: layout(0, deliveredRow) },
    eat: { sheet: `${ART_ROOT}/animals/rabbit/eat.png`, sheetWidth: 2048, sheetHeight: 256, frames: layout(0, deliveredRow), durationsMs: Array(8).fill(83) },
  },
  hamster: {
    portrait: `${ART_ROOT}/portraits/hamster.png`,
    idle: { sheet: `${ART_ROOT}/animals/hamster/actions.png`, sheetWidth: 2048, sheetHeight: 512, frames: layout(256, deliveredRow) },
    eat: { sheet: `${ART_ROOT}/animals/hamster/actions.png`, sheetWidth: 2048, sheetHeight: 512, frames: layout(0, deliveredRow), durationsMs: Array(8).fill(125) },
  },
  "red-panda": {
    portrait: `${ART_ROOT}/portraits/red-panda.png`,
    idle: { sheet: `${ART_ROOT}/animals/red-panda/actions.png`, sheetWidth: 2048, sheetHeight: 512, frames: layout(256, deliveredRow) },
    eat: { sheet: `${ART_ROOT}/animals/red-panda/actions.png`, sheetWidth: 2048, sheetHeight: 512, frames: layout(0, deliveredRow), durationsMs: Array(8).fill(125) },
  },
  capybara: {
    portrait: `${ART_ROOT}/portraits/capybara.png`,
    idle: { sheet: `${ART_ROOT}/animals/capybara/actions.png`, sheetWidth: 2048, sheetHeight: 512, frames: layout(256, deliveredRow) },
    eat: {
      sheet: `${ART_ROOT}/animals/capybara/actions.png`,
      sheetWidth: 2048,
      sheetHeight: 512,
      frames: layout(0, [0, 256, 512, 768, 1024, 768, 512, 256]),
      durationsMs: Array(8).fill(125),
    },
  },
  deer: {
    portrait: `${ART_ROOT}/portraits/deer.png`,
    idle: { sheet: `${ART_ROOT}/animals/deer/actions.png`, sheetWidth: 2048, sheetHeight: 512, frames: layout(0, deliveredRow) },
    eat: { sheet: `${ART_ROOT}/animals/deer/actions.png`, sheetWidth: 2048, sheetHeight: 512, frames: layout(256, deliveredRow), durationsMs: Array(8).fill(125) },
  },
};

export const SCENE_ART = {
  // The flat v0.2 land is packaged with the desktop app.
  base: "/art/v0.2.0/scene/scene-land-flat-base@2x.png",
  warehouse: `${ART_ROOT}/scene/scene-warehouse-idle@2x.png`,
  landTiles: {
    grass: `${ART_ROOT}/scene/land-tile-grass@2x.png`,
    tilled: `${ART_ROOT}/scene/land-tile-tilled@2x.png`,
  },
  plants: {
    clover: { seedling: `${ART_ROOT}/plants/clover/plant-clover-seedling@2x.png`, full: `${ART_ROOT}/plants/clover/plant-clover-full-v2@2x.png`, compendium: `${ART_ROOT}/plants/clover/plant-clover-full-v2-square@2x.png` },
    sunflower: { seedling: `${ART_ROOT}/plants/sunflower/plant-sunflower-seedling@2x.png`, full: `${ART_ROOT}/plants/sunflower/plant-sunflower-full@2x.png`, compendium: `${ART_ROOT}/plants/sunflower/plant-sunflower-full@2x.png` },
    bamboo: { seedling: `${ART_ROOT}/plants/bamboo/plant-bamboo-seedling@2x.png`, full: `${ART_ROOT}/plants/bamboo/plant-bamboo-full@2x.png`, compendium: `${ART_ROOT}/plants/bamboo/plant-bamboo-full@2x.png` },
    corn: { seedling: `${ART_ROOT}/plants/corn/plant-corn-seedling@2x.png`, full: `${ART_ROOT}/plants/corn/plant-corn-full@2x.png`, compendium: `${ART_ROOT}/plants/corn/plant-corn-full@2x.png` },
    apple: { seedling: `${ART_ROOT}/plants/apple/plant-apple-seedling@2x.png`, full: `${ART_ROOT}/plants/apple/plant-apple-full@2x.png`, compendium: `${ART_ROOT}/plants/apple/plant-apple-full@2x.png` },
  },
} as const;

export type ScenePoint = readonly [number, number];
export type ScenePolygon = readonly ScenePoint[];
export type PlotId = "plot1" | "plot2" | "plot3" | "plot4" | "plot5";
export type PlantSlotId = "A" | "B" | "C" | "D" | "E";
type LandPlot = {
  species: string;
  renderRect: readonly [number, number, number, number];
  plantSlots: Record<PlantSlotId, ScenePoint>;
  animalFootSafePolygon: ScenePolygon;
  adjacentPlots: readonly PlotId[];
};
type LandLayout = {
  coordinateSpace: { canvas: readonly [number, number] };
  presentation: { baseScale: number; tightCropRect: readonly [number, number, number, number]; runtimeViewport: readonly [number, number] };
  warehouse: { renderRect: readonly [number, number, number, number]; visualScale: number; hitPolygon: ScenePolygon; animalForbiddenPolygon: ScenePolygon };
  windowDragControl: { hitRect: readonly [number, number, number, number]; visualSize: readonly [number, number] };
  windowHideControl: { hitRect: readonly [number, number, number, number]; visualSize: readonly [number, number] };
  plantRendering: { seedlingWidthOfAuthoringCanvas: number; matureWidthOfAuthoringCanvas: number };
  plantSlotPattern: { visibleCount: Record<"1" | "2" | "3" | "4" | "5", readonly PlantSlotId[]> };
  plots: Record<PlotId, LandPlot>;
  animalMovementClip: { connectors: Record<string, ScenePolygon> };
};

export const LAND_LAYOUT = landLayoutJson as unknown as LandLayout;
export const UI_TOKENS = uiTokensJson;
export const PLOT_IDS = ["plot1", "plot2", "plot3", "plot4", "plot5"] as const;
export const PLOT_SLOTS = PLOT_IDS.map((id) => {
  const [x, y, w, h] = LAND_LAYOUT.plots[id].renderRect;
  return { id, x, y, w, h, ...LAND_LAYOUT.plots[id] };
});
export const ANIMAL_MOVEMENT_POLYGONS: readonly ScenePolygon[] = [
  ...PLOT_IDS.map((id) => LAND_LAYOUT.plots[id].animalFootSafePolygon),
  ...Object.values(LAND_LAYOUT.animalMovementClip.connectors),
];

export const ICON_ART = {
  warehouse: "/ui/icons/icon-nav-warehouse.svg",
  buy: "/ui/icons/icon-nav-buy.svg",
  sell: "/ui/icons/icon-nav-sell.svg",
  catalog: "/ui/icons/icon-nav-compendium.svg",
  stats: "/ui/icons/icon-nav-statistics.svg",
  notice: "/ui/icons/icon-nav-notice.svg",
  coin: "/ui/icons/icon-currency-coin.svg",
  plant: "/ui/icons/icon-category-plant.svg",
  animal: "/ui/icons/icon-category-animal.svg",
  close: "/ui/icons/icon-action-close.svg",
  minus: "/ui/icons/icon-action-minus.svg",
  plus: "/ui/icons/icon-action-plus.svg",
  previous: "/ui/icons/icon-action-previous.svg",
  next: "/ui/icons/icon-action-next.svg",
  windowDrag: "/ui/icons/icon-window-drag.svg",
} as const;

export const PLACEHOLDER_ASSETS = {
  plants: [],
  interface: ["coin-emoji", "input-emoji", "permission-logo-emoji"],
} as const;

export type Quantities = Record<string, number>;
export type TierSnapshot = {
  plantCount: string;
  stockCent: string;
  plantRateCentPerMinute: string;
  totalRateCentPerMinute: string;
  nextSeedPriceCoins: string;
  plantDiscovered: boolean;
  animalCount: string;
  animalPurchasePriceCoins: string;
  biteCent: string;
  groupBiteCent: string;
  growthPerFeedCent: string;
  growthCent: string;
  feedingThreshold: number;
  feedingProgress: number;
  lifetimePaidPurchaseCount: string;
  singleSaleValueCoins: string | null;
  animalDiscovered: boolean;
};
export type StatisticsSnapshot = {
  localDate: string;
  todayProductiveSeconds: string;
  todayInputs: string;
  lifetimeProductiveSeconds: string;
  lifetimeInputs: string;
  productiveDays: string;
};
export type EconomySnapshot = {
  coins: string;
  lifetimeInputs: string;
  statistics: StatisticsSnapshot;
  tiers: Record<string, TierSnapshot>;
  saveStatus: "new" | "loaded" | "recovered";
};

export function compactNumber(value: string): string {
  const number = BigInt(value || "0");
  if (number < 1_000n) return number.toLocaleString("zh-CN");
  if (number < 1_000_000n) return `${number / 1_000n}.${number / 100n % 10n}K`;
  return `${number / 1_000_000n}.${number / 100_000n % 10n}M`;
}

export function formatCent(value: string): string {
  const cent = BigInt(value || "0");
  const whole = cent / 100n;
  const fraction = cent % 100n;
  if (whole >= 1_000n && fraction === 0n) return compactNumber(whole.toString());
  return `${whole.toLocaleString("zh-CN")}.${fraction.toString().padStart(2, "0")}`;
}

export function formatRatePerSecond(rateCentPerMinute: string): string {
  const numerator = BigInt(rateCentPerMinute || "0");
  const hundredthsPerSecond = numerator / 60n;
  return `${compactNumber((hundredthsPerSecond / 100n).toString())}.${(hundredthsPerSecond % 100n).toString().padStart(2, "0")}/s`;
}

export function formatDuration(secondsText: string): string {
  const seconds = BigInt(secondsText || "0");
  const hours = seconds / 3_600n;
  const minutes = seconds % 3_600n / 60n;
  return `${hours.toLocaleString("zh-CN")}h ${minutes.toString().padStart(2, "0")}min`;
}

export function quantityLimit(value: string): number {
  const parsed = BigInt(value || "0");
  const safeMaximum = BigInt(Number.MAX_SAFE_INTEGER);
  return parsed > safeMaximum ? Number.MAX_SAFE_INTEGER : Number(parsed);
}
