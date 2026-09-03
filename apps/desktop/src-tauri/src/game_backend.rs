use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Mutex,
    time::Duration,
};

use clickclackfarm_content_config::load_mvp_content;
use clickclackfarm_domain::{
    AnimalId, AnimalState, Coins, CollectionState, ContentConfig, EntityCount, FeedingProgress,
    GameState, PlantId, PlantState, ProductionRemainder60, PurchaseKind, PurchaseSelection,
    SaleSelection, Wallet, apply_effective_inputs, apply_purchase_batch, apply_sale_batch,
    quote_purchase_batch, quote_sale_batch, settle_production,
};
use num_bigint::BigUint;
use serde::{Deserialize, Serialize};

const DEMO_INITIAL_COINS: u64 = 100;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseRequest {
    pub plants: BTreeMap<String, u64>,
    pub animals: BTreeMap<String, u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaleRequest {
    pub animals: BTreeMap<String, u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeQuote {
    pub total: String,
    pub emergency_free: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TierSnapshot {
    pub plant_count: String,
    pub stock_cent: String,
    pub plant_rate_cent_per_minute: String,
    pub total_rate_cent_per_minute: String,
    pub next_seed_price_coins: String,
    pub plant_discovered: bool,
    pub animal_count: String,
    pub animal_purchase_price_coins: String,
    pub bite_cent: String,
    pub group_bite_cent: String,
    pub growth_per_feed_cent: String,
    pub growth_cent: String,
    pub feeding_threshold: u8,
    pub feeding_progress: u8,
    pub lifetime_paid_purchase_count: String,
    pub single_sale_value_coins: Option<String>,
    pub animal_discovered: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatisticsSnapshot {
    pub local_date: String,
    pub today_productive_seconds: String,
    pub today_inputs: String,
    pub lifetime_productive_seconds: String,
    pub lifetime_inputs: String,
    pub productive_days: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EconomySnapshot {
    pub coins: String,
    pub lifetime_inputs: String,
    pub statistics: StatisticsSnapshot,
    pub tiers: BTreeMap<String, TierSnapshot>,
    pub save_status: &'static str,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveFile {
    version: u8,
    content_version: String,
    coins: String,
    lifetime_inputs: String,
    plants: BTreeMap<String, SavedPlant>,
    animals: BTreeMap<String, SavedAnimal>,
    discovered_plants: Vec<String>,
    discovered_animals: Vec<String>,
    #[serde(default)]
    statistics: SavedStatistics,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedPlant {
    count: String,
    stock_cent: String,
    production_remainder_60: u8,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedAnimal {
    count: String,
    total_growth_cent: String,
    feeding_progress: u8,
    lifetime_paid_purchase_count: String,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedStatistics {
    #[serde(default)]
    current_local_date: String,
    #[serde(default)]
    production_subsecond_nanos: u32,
    #[serde(default)]
    days: BTreeMap<String, SavedDayStatistics>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedDayStatistics {
    #[serde(default = "zero_text")]
    productive_nanos: String,
    #[serde(default = "zero_text")]
    inputs: String,
}

#[derive(Clone, Debug, Default)]
struct DayStatistics {
    productive_nanos: u64,
    inputs: BigUint,
}

#[derive(Clone, Debug, Default)]
struct Statistics {
    current_local_date: String,
    production_subsecond_nanos: u32,
    days: BTreeMap<String, DayStatistics>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CalendarSample {
    date: String,
    seconds_since_midnight: u32,
}

struct Inner {
    state: GameState,
    lifetime_inputs: BigUint,
    statistics: Statistics,
    last_calendar: Option<CalendarSample>,
    consumed_global_inputs: u64,
    save_status: &'static str,
}

pub struct GameEngine {
    content: ContentConfig,
    save_path: PathBuf,
    inner: Mutex<Inner>,
}

impl GameEngine {
    pub fn load(save_path: PathBuf) -> Result<Self, String> {
        let content = load_mvp_content().map_err(|error| error.to_string())?;
        let (state, lifetime_inputs, statistics, save_status) =
            match load_save(&save_path, &content) {
                Ok(Some(value)) => (value.0, value.1, value.2, "loaded"),
                Ok(None) => (
                    new_demo_game_state(&content)?,
                    BigUint::default(),
                    Statistics::default(),
                    "new",
                ),
                Err(_) => (
                    new_demo_game_state(&content)?,
                    BigUint::default(),
                    Statistics::default(),
                    "recovered",
                ),
            };
        let last_calendar = local_calendar_sample();
        Ok(Self {
            content,
            save_path,
            inner: Mutex::new(Inner {
                state,
                lifetime_inputs,
                statistics,
                last_calendar,
                consumed_global_inputs: 0,
                save_status,
            }),
        })
    }

    pub fn snapshot(
        &self,
        total_global_inputs: u64,
        productive: Duration,
    ) -> Result<EconomySnapshot, String> {
        let mut inner = self.lock()?;
        let calendar = local_calendar_sample();
        let changed = self.settle(
            &mut inner,
            total_global_inputs,
            productive,
            calendar.as_ref(),
        )?;
        if changed {
            self.save(&inner)?;
        }
        to_snapshot(&inner, &self.content, calendar.as_ref())
    }

    pub fn quote_purchase(&self, request: &PurchaseRequest) -> Result<TradeQuote, String> {
        let inner = self.lock()?;
        let quote =
            quote_purchase_batch(&inner.state, &self.content, &purchase_selections(request)?)
                .map_err(domain_error)?;
        Ok(TradeQuote {
            total: quote.total_cost.to_decimal_string(),
            emergency_free: quote.lines.iter().any(|line| line.emergency_free),
        })
    }

    pub fn purchase(&self, request: &PurchaseRequest) -> Result<EconomySnapshot, String> {
        let mut inner = self.lock()?;
        let transition =
            apply_purchase_batch(&inner.state, &self.content, &purchase_selections(request)?)
                .map_err(domain_error)?;
        inner.state = transition.state;
        self.save(&inner)?;
        to_snapshot(&inner, &self.content, local_calendar_sample().as_ref())
    }

    pub fn quote_sale(&self, request: &SaleRequest) -> Result<TradeQuote, String> {
        let inner = self.lock()?;
        let quote = quote_sale_batch(&inner.state, &self.content, &sale_selections(request)?)
            .map_err(domain_error)?;
        Ok(TradeQuote {
            total: quote.total_coins.to_decimal_string(),
            emergency_free: false,
        })
    }

    pub fn sale(&self, request: &SaleRequest) -> Result<EconomySnapshot, String> {
        let mut inner = self.lock()?;
        let transition = apply_sale_batch(&inner.state, &self.content, &sale_selections(request)?)
            .map_err(domain_error)?;
        inner.state = transition.state;
        self.save(&inner)?;
        to_snapshot(&inner, &self.content, local_calendar_sample().as_ref())
    }

    #[cfg(test)]
    pub fn reset(&self, total_global_inputs: u64) -> Result<EconomySnapshot, String> {
        let mut inner = self.lock()?;
        inner.state = new_demo_game_state(&self.content)?;
        inner.lifetime_inputs = BigUint::default();
        inner.statistics = Statistics::default();
        inner.last_calendar = local_calendar_sample();
        inner.consumed_global_inputs = total_global_inputs;
        inner.save_status = "new";
        self.save(&inner)?;
        to_snapshot(&inner, &self.content, inner.last_calendar.as_ref())
    }

    fn settle(
        &self,
        inner: &mut Inner,
        total_global_inputs: u64,
        productive: Duration,
        calendar: Option<&CalendarSample>,
    ) -> Result<bool, String> {
        let mut changed = self.settle_productive(inner, productive, calendar)?;

        if total_global_inputs >= inner.consumed_global_inputs {
            let delta = total_global_inputs - inner.consumed_global_inputs;
            inner.consumed_global_inputs = total_global_inputs;
            if delta > 0 {
                let transition =
                    apply_effective_inputs(&inner.state, &self.content, &BigUint::from(delta))
                        .map_err(domain_error)?;
                inner.state = transition.state;
                inner.lifetime_inputs += delta;
                record_inputs(&mut inner.statistics, calendar, delta);
                changed = true;
            }
        } else {
            inner.consumed_global_inputs = total_global_inputs;
        }
        inner.last_calendar = calendar.cloned();
        Ok(changed)
    }

    fn settle_productive(
        &self,
        inner: &mut Inner,
        productive: Duration,
        calendar: Option<&CalendarSample>,
    ) -> Result<bool, String> {
        let nanos = u64::try_from(productive.as_nanos()).unwrap_or(u64::MAX);
        if nanos == 0 || calendar.is_none() {
            return Ok(false);
        }
        record_productive(
            &mut inner.statistics,
            inner.last_calendar.as_ref(),
            calendar.expect("checked above"),
            nanos,
        );
        let total_nanos =
            u64::from(inner.statistics.production_subsecond_nanos).saturating_add(nanos);
        let whole_seconds = total_nanos / 1_000_000_000;
        inner.statistics.production_subsecond_nanos =
            u32::try_from(total_nanos % 1_000_000_000).expect("subsecond remainder fits u32");
        if whole_seconds > 0 {
            let transition =
                settle_production(&inner.state, &self.content, &BigUint::from(whole_seconds))
                    .map_err(domain_error)?;
            inner.state = transition.state;
        }
        Ok(true)
    }

    fn save(&self, inner: &Inner) -> Result<(), String> {
        if let Some(parent) = self.save_path.parent() {
            fs::create_dir_all(parent).map_err(|_| "无法创建存档目录".to_owned())?;
        }
        let bytes = serde_json::to_vec_pretty(&SaveFile::from_inner(inner))
            .map_err(|_| "无法生成存档".to_owned())?;
        fs::write(&self.save_path, bytes).map_err(|_| "无法写入存档".to_owned())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Inner>, String> {
        self.inner
            .lock()
            .map_err(|_| "游戏状态暂时不可用".to_owned())
    }
}

fn new_demo_game_state(content: &ContentConfig) -> Result<GameState, String> {
    GameState::new(
        content,
        Coins::from(DEMO_INITIAL_COINS),
        [(
            PlantId::new("clover").map_err(domain_error)?,
            EntityCount::from(1_u64),
        )],
        [(
            AnimalId::new("rabbit").map_err(domain_error)?,
            EntityCount::from(1_u64),
        )],
    )
    .map_err(domain_error)
}

fn purchase_selections(request: &PurchaseRequest) -> Result<Vec<PurchaseSelection>, String> {
    let plants = request
        .plants
        .iter()
        .filter(|(_, quantity)| **quantity > 0)
        .map(|(id, quantity)| {
            Ok(PurchaseSelection {
                kind: PurchaseKind::Plant(PlantId::new(id).map_err(domain_error)?),
                quantity: EntityCount::from(*quantity),
            })
        });
    let animals = request
        .animals
        .iter()
        .filter(|(_, quantity)| **quantity > 0)
        .map(|(id, quantity)| {
            Ok(PurchaseSelection {
                kind: PurchaseKind::Animal(AnimalId::new(id).map_err(domain_error)?),
                quantity: EntityCount::from(*quantity),
            })
        });
    plants.chain(animals).collect()
}

fn sale_selections(request: &SaleRequest) -> Result<Vec<SaleSelection>, String> {
    request
        .animals
        .iter()
        .filter(|(_, quantity)| **quantity > 0)
        .map(|(id, quantity)| {
            Ok(SaleSelection {
                animal_id: AnimalId::new(id).map_err(domain_error)?,
                quantity: EntityCount::from(*quantity),
            })
        })
        .collect()
}

fn to_snapshot(
    inner: &Inner,
    content: &ContentConfig,
    calendar: Option<&CalendarSample>,
) -> Result<EconomySnapshot, String> {
    let mut tiers = BTreeMap::new();
    for (plant_id, plant) in inner.state.plants() {
        let plant_config = content.plant(plant_id).expect("validated plant");
        let animal_id = &plant_config.paired_animal_id;
        let animal = inner.state.animals().get(animal_id);
        let animal = animal.expect("validated MVP state contains paired animal");
        let animal_config = content.animal(animal_id).expect("validated paired animal");
        let next_seed_price = quote_purchase_batch(
            &inner.state,
            content,
            &[PurchaseSelection {
                kind: PurchaseKind::Plant(plant_id.clone()),
                quantity: EntityCount::from(1_u64),
            }],
        )
        .map_err(domain_error)?
        .total_cost;
        let single_sale_value = if animal.count.is_zero() {
            None
        } else {
            Some(
                quote_sale_batch(
                    &inner.state,
                    content,
                    &[SaleSelection {
                        animal_id: animal_id.clone(),
                        quantity: EntityCount::from(1_u64),
                    }],
                )
                .map_err(domain_error)?
                .total_coins
                .to_decimal_string(),
            )
        };
        tiers.insert(
            plant_id.to_string(),
            TierSnapshot {
                plant_count: plant.count.to_decimal_string(),
                stock_cent: plant.stock_cent.to_decimal_string(),
                plant_rate_cent_per_minute: plant_config.rate_cent_per_minute.to_decimal_string(),
                total_rate_cent_per_minute: (plant.count.as_biguint()
                    * plant_config.rate_cent_per_minute.as_biguint())
                .to_str_radix(10),
                next_seed_price_coins: next_seed_price.to_decimal_string(),
                plant_discovered: inner.state.collection().is_plant_discovered(plant_id),
                animal_count: animal.count.to_decimal_string(),
                animal_purchase_price_coins: animal_config
                    .fixed_purchase_price_coins
                    .to_decimal_string(),
                bite_cent: animal_config.bite_cent.to_decimal_string(),
                group_bite_cent: (animal.count.as_biguint() * animal_config.bite_cent.as_biguint())
                    .to_str_radix(10),
                growth_per_feed_cent: animal_config.bite_cent.to_decimal_string(),
                growth_cent: animal.total_growth_cent.to_decimal_string(),
                feeding_threshold: animal_config.feeding_threshold,
                feeding_progress: animal.feeding_progress.value(),
                lifetime_paid_purchase_count: animal
                    .lifetime_paid_purchase_count
                    .to_decimal_string(),
                single_sale_value_coins: single_sale_value,
                animal_discovered: inner.state.collection().is_animal_discovered(animal_id),
            },
        );
    }
    Ok(EconomySnapshot {
        coins: inner.state.wallet().coins.to_decimal_string(),
        lifetime_inputs: inner.lifetime_inputs.to_str_radix(10),
        statistics: statistics_snapshot(inner, calendar),
        tiers,
        save_status: inner.save_status,
    })
}

fn statistics_snapshot(inner: &Inner, calendar: Option<&CalendarSample>) -> StatisticsSnapshot {
    let date = calendar
        .map(|sample| sample.date.clone())
        .filter(|date| !date.is_empty())
        .unwrap_or_else(|| inner.statistics.current_local_date.clone());
    let today = inner
        .statistics
        .days
        .get(&date)
        .cloned()
        .unwrap_or_default();
    let lifetime_productive_nanos = inner
        .statistics
        .days
        .values()
        .fold(0_u64, |sum, day| sum.saturating_add(day.productive_nanos));
    let productive_days = inner
        .statistics
        .days
        .values()
        .filter(|day| day.productive_nanos >= 1_000_000_000)
        .count();
    StatisticsSnapshot {
        local_date: date,
        today_productive_seconds: (today.productive_nanos / 1_000_000_000).to_string(),
        today_inputs: today.inputs.to_str_radix(10),
        lifetime_productive_seconds: (lifetime_productive_nanos / 1_000_000_000).to_string(),
        lifetime_inputs: inner.lifetime_inputs.to_str_radix(10),
        productive_days: productive_days.to_string(),
    }
}

impl SaveFile {
    fn from_inner(inner: &Inner) -> Self {
        Self {
            version: 1,
            content_version: inner.state.content_version().to_owned(),
            coins: inner.state.wallet().coins.to_decimal_string(),
            lifetime_inputs: inner.lifetime_inputs.to_str_radix(10),
            plants: inner
                .state
                .plants()
                .iter()
                .map(|(id, state)| {
                    (
                        id.to_string(),
                        SavedPlant {
                            count: state.count.to_decimal_string(),
                            stock_cent: state.stock_cent.to_decimal_string(),
                            production_remainder_60: state.production_remainder_60.value(),
                        },
                    )
                })
                .collect(),
            animals: inner
                .state
                .animals()
                .iter()
                .map(|(id, state)| {
                    (
                        id.to_string(),
                        SavedAnimal {
                            count: state.count.to_decimal_string(),
                            total_growth_cent: state.total_growth_cent.to_decimal_string(),
                            feeding_progress: state.feeding_progress.value(),
                            lifetime_paid_purchase_count: state
                                .lifetime_paid_purchase_count
                                .to_decimal_string(),
                        },
                    )
                })
                .collect(),
            discovered_plants: inner
                .state
                .collection()
                .discovered_plants()
                .iter()
                .map(ToString::to_string)
                .collect(),
            discovered_animals: inner
                .state
                .collection()
                .discovered_animals()
                .iter()
                .map(ToString::to_string)
                .collect(),
            statistics: SavedStatistics {
                current_local_date: inner.statistics.current_local_date.clone(),
                production_subsecond_nanos: inner.statistics.production_subsecond_nanos,
                days: inner
                    .statistics
                    .days
                    .iter()
                    .map(|(date, day)| {
                        (
                            date.clone(),
                            SavedDayStatistics {
                                productive_nanos: day.productive_nanos.to_string(),
                                inputs: day.inputs.to_str_radix(10),
                            },
                        )
                    })
                    .collect(),
            },
        }
    }
}

fn load_save(
    path: &Path,
    content: &ContentConfig,
) -> Result<Option<(GameState, BigUint, Statistics)>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let file: SaveFile =
        serde_json::from_slice(&fs::read(path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    if file.version != 1 {
        return Err("unsupported save".into());
    }
    let plants = file
        .plants
        .into_iter()
        .map(|(id, state)| {
            Ok((
                PlantId::new(id).map_err(domain_error)?,
                PlantState {
                    count: parse_amount(&state.count)?,
                    stock_cent: parse_amount(&state.stock_cent)?,
                    production_remainder_60: ProductionRemainder60::try_new(
                        state.production_remainder_60,
                    )
                    .map_err(domain_error)?,
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    let animals = file
        .animals
        .into_iter()
        .map(|(id, state)| {
            Ok((
                AnimalId::new(id).map_err(domain_error)?,
                AnimalState {
                    count: parse_amount(&state.count)?,
                    total_growth_cent: parse_amount(&state.total_growth_cent)?,
                    feeding_progress: FeedingProgress::new_unchecked(state.feeding_progress),
                    lifetime_paid_purchase_count: parse_amount(
                        &state.lifetime_paid_purchase_count,
                    )?,
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    let collection = CollectionState::from_discovered(
        file.discovered_plants
            .into_iter()
            .map(PlantId::new)
            .collect::<Result<Vec<_>, _>>()
            .map_err(domain_error)?,
        file.discovered_animals
            .into_iter()
            .map(AnimalId::new)
            .collect::<Result<Vec<_>, _>>()
            .map_err(domain_error)?,
    );
    let statistics = statistics_from_saved(file.statistics)?;
    let state = GameState::try_from_parts(
        file.content_version,
        Wallet {
            coins: parse_amount(&file.coins)?,
        },
        plants,
        animals,
        collection,
        content,
    )
    .map_err(domain_error)?;
    Ok(Some((
        state,
        parse_biguint(&file.lifetime_inputs)?,
        statistics,
    )))
}

fn statistics_from_saved(saved: SavedStatistics) -> Result<Statistics, String> {
    Ok(Statistics {
        current_local_date: saved.current_local_date,
        production_subsecond_nanos: saved.production_subsecond_nanos.min(999_999_999),
        days: saved
            .days
            .into_iter()
            .map(|(date, day)| {
                Ok((
                    date,
                    DayStatistics {
                        productive_nanos: day
                            .productive_nanos
                            .parse::<u64>()
                            .map_err(|_| "存档统计数值无效".to_owned())?,
                        inputs: parse_biguint(&day.inputs)?,
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?,
    })
}

fn record_inputs(statistics: &mut Statistics, calendar: Option<&CalendarSample>, count: u64) {
    let Some(calendar) = calendar else {
        return;
    };
    statistics.current_local_date = calendar.date.clone();
    statistics
        .days
        .entry(calendar.date.clone())
        .or_default()
        .inputs += count;
}

fn record_productive(
    statistics: &mut Statistics,
    previous: Option<&CalendarSample>,
    current: &CalendarSample,
    productive_nanos: u64,
) {
    statistics.current_local_date = current.date.clone();
    if let Some(previous) = previous
        && previous.date != current.date
    {
        let nanos_until_midnight =
            u64::from(86_400_u32.saturating_sub(previous.seconds_since_midnight))
                .saturating_mul(1_000_000_000);
        let previous_nanos = productive_nanos.min(nanos_until_midnight);
        add_productive_nanos(statistics, &previous.date, previous_nanos);
        add_productive_nanos(
            statistics,
            &current.date,
            productive_nanos.saturating_sub(previous_nanos),
        );
    } else {
        add_productive_nanos(statistics, &current.date, productive_nanos);
    }
}

fn add_productive_nanos(statistics: &mut Statistics, date: &str, nanos: u64) {
    let day = statistics.days.entry(date.to_owned()).or_default();
    day.productive_nanos = day.productive_nanos.saturating_add(nanos);
}

fn local_calendar_sample() -> Option<CalendarSample> {
    let mut timestamp: libc::time_t = 0;
    // SAFETY: timestamp and local are valid writable values for libc.
    if unsafe { libc::time(&mut timestamp) } < 0 {
        return None;
    }
    let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
    #[cfg(unix)]
    {
        // SAFETY: localtime_r initializes local when it returns non-null.
        if unsafe { libc::localtime_r(&timestamp, local.as_mut_ptr()) }.is_null() {
            return None;
        }
    }
    #[cfg(target_os = "windows")]
    {
        // SAFETY: localtime_s initializes local when it returns zero.
        if unsafe { libc::localtime_s(local.as_mut_ptr(), &timestamp) } != 0 {
            return None;
        }
    }
    // SAFETY: checked successful initialization above.
    let local = unsafe { local.assume_init() };
    let year = local.tm_year.checked_add(1900)?;
    let month = local.tm_mon.checked_add(1)?;
    let seconds_since_midnight = u32::try_from(local.tm_hour).ok()? * 3_600
        + u32::try_from(local.tm_min).ok()? * 60
        + u32::try_from(local.tm_sec).ok()?;
    Some(CalendarSample {
        date: format!("{year:04}-{month:02}-{:02}", local.tm_mday),
        seconds_since_midnight,
    })
}

fn zero_text() -> String {
    "0".to_owned()
}

fn parse_biguint(value: &str) -> Result<BigUint, String> {
    BigUint::from_str(value).map_err(|_| "存档数值无效".to_owned())
}

fn parse_amount<T: From<BigUint>>(value: &str) -> Result<T, String> {
    Ok(T::from(parse_biguint(value)?))
}

fn domain_error(error: impl std::fmt::Display) -> String {
    let text = error.to_string();
    if text.contains("insufficient coins") {
        "金币不足".to_owned()
    } else if text.contains("empty selection") {
        "请至少选择一项".to_owned()
    } else if text.contains("exceeds owned") {
        "出售数量超过持有数量".to_owned()
    } else {
        format!("操作失败：{text}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::{
        sync::atomic::{AtomicU64, Ordering},
        time::Duration,
    };

    static NEXT_TEST_FILE: AtomicU64 = AtomicU64::new(0);

    struct TestSave(PathBuf);

    impl TestSave {
        fn new(name: &str) -> Self {
            let sequence = NEXT_TEST_FILE.fetch_add(1, Ordering::Relaxed);
            Self(std::env::temp_dir().join(format!(
                "clickclackfarm-{name}-{}-{sequence}.json",
                std::process::id()
            )))
        }
    }

    impl Drop for TestSave {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    #[test]
    fn new_save_starts_with_release_assets() {
        let save = TestSave::new("v1-initial-assets");
        let engine = GameEngine::load(save.0.clone()).expect("load game");
        let snapshot = engine.snapshot(0, Duration::ZERO).expect("snapshot");

        assert_eq!(snapshot.coins, "100");
        assert_eq!(snapshot.tiers["clover"].plant_count, "1");
        assert_eq!(snapshot.tiers["clover"].animal_count, "1");
    }

    #[test]
    fn input_is_counted_once_and_persisted() {
        let save = TestSave::new("persisted-input");
        let engine = GameEngine::load(save.0.clone()).expect("load demo");
        let snapshot = engine.snapshot(1, Duration::ZERO).expect("record input");
        assert_eq!(snapshot.lifetime_inputs, "1");
        assert_eq!(snapshot.statistics.today_inputs, "1");

        let restored = GameEngine::load(save.0.clone()).expect("restore demo");
        let restored = restored
            .snapshot(0, Duration::ZERO)
            .expect("restored snapshot");
        assert_eq!(restored.lifetime_inputs, "1");
        assert_eq!(restored.statistics.today_inputs, "1");
    }

    #[test]
    fn global_input_delta_is_counted_only_once() {
        let save = TestSave::new("global-input");
        let engine = GameEngine::load(save.0.clone()).expect("load demo");
        let calendar = CalendarSample {
            date: "2026-08-30".to_owned(),
            seconds_since_midnight: 12_000,
        };
        {
            let mut inner = engine.lock().expect("lock engine");
            inner.last_calendar = Some(calendar.clone());
            assert!(
                engine
                    .settle(&mut inner, 7, Duration::ZERO, Some(&calendar))
                    .expect("first settlement")
            );
            assert!(
                !engine
                    .settle(&mut inner, 7, Duration::ZERO, Some(&calendar))
                    .expect("second settlement")
            );
            assert_eq!(inner.lifetime_inputs, BigUint::from(7_u8));
            assert_eq!(
                statistics_snapshot(&inner, Some(&calendar)).today_inputs,
                "7"
            );
        }
    }

    #[test]
    fn productive_time_is_split_across_local_midnight() {
        let mut statistics = Statistics::default();
        let previous = CalendarSample {
            date: "2026-08-29".to_owned(),
            seconds_since_midnight: 86_399,
        };
        let current = CalendarSample {
            date: "2026-08-30".to_owned(),
            seconds_since_midnight: 1,
        };

        record_productive(&mut statistics, Some(&previous), &current, 2_000_000_000);

        assert_eq!(
            statistics.days["2026-08-29"].productive_nanos,
            1_000_000_000
        );
        assert_eq!(
            statistics.days["2026-08-30"].productive_nanos,
            1_000_000_000
        );
        assert_eq!(statistics.current_local_date, "2026-08-30");
    }

    #[test]
    fn zero_productive_duration_does_not_produce() {
        let save = TestSave::new("inactive");
        let engine = GameEngine::load(save.0.clone()).expect("load demo");
        let before = engine.snapshot(0, Duration::ZERO).expect("before");
        let after = engine.snapshot(0, Duration::ZERO).expect("after");

        assert_eq!(
            before.tiers["clover"].stock_cent,
            after.tiers["clover"].stock_cent
        );
        assert_eq!(after.statistics.lifetime_productive_seconds, "0");
    }

    #[test]
    fn legacy_save_without_statistics_still_loads() {
        let save = TestSave::new("legacy-save");
        let engine = GameEngine::load(save.0.clone()).expect("load demo");
        engine
            .snapshot(1, Duration::ZERO)
            .expect("save current format");

        let mut document: Value =
            serde_json::from_slice(&fs::read(&save.0).expect("read save")).expect("parse save");
        document
            .as_object_mut()
            .expect("save object")
            .remove("statistics");
        fs::write(
            &save.0,
            serde_json::to_vec_pretty(&document).expect("serialize legacy"),
        )
        .expect("write legacy save");

        let restored = GameEngine::load(save.0.clone()).expect("load legacy save");
        let snapshot = restored.snapshot(0, Duration::ZERO).expect("snapshot");
        assert_eq!(snapshot.coins, DEMO_INITIAL_COINS.to_string());
        assert_eq!(snapshot.lifetime_inputs, "1");
        assert_eq!(snapshot.statistics.today_inputs, "0");
    }

    #[test]
    fn snapshot_uses_authoritative_single_animal_sale_quote() {
        let save = TestSave::new("sale-quote");
        let engine = GameEngine::load(save.0.clone()).expect("load demo");
        let request = SaleRequest {
            animals: BTreeMap::from([("rabbit".to_owned(), 1)]),
        };
        let quote = engine.quote_sale(&request).expect("quote rabbit sale");
        let snapshot = engine.snapshot(0, Duration::ZERO).expect("snapshot");

        assert_eq!(
            snapshot.tiers["clover"].single_sale_value_coins.as_deref(),
            Some(quote.total.as_str())
        );
    }

    #[test]
    fn reset_restores_demo_state_and_clears_statistics() {
        let save = TestSave::new("reset");
        let engine = GameEngine::load(save.0.clone()).expect("load demo");
        engine
            .snapshot(1, Duration::from_secs(1))
            .expect("mutate demo");

        let snapshot = engine.reset(0).expect("reset demo");
        assert_eq!(snapshot.coins, DEMO_INITIAL_COINS.to_string());
        assert_eq!(snapshot.tiers["clover"].plant_count, "1");
        assert_eq!(snapshot.tiers["clover"].animal_count, "1");
        assert_eq!(snapshot.statistics.lifetime_inputs, "0");
        assert_eq!(snapshot.statistics.lifetime_productive_seconds, "0");
    }
}
