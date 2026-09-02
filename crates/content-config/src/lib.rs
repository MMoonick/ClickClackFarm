//! Bundled MVP content for Click Clack Farm.
//!
//! `load_mvp_content` performs structural and numerical validation every time it
//! constructs the catalog. The desktop bootstrap should fail closed if this
//! function returns an error.

use clickclackfarm_domain::{
    AnimalConfig, AnimalId, Coins, ContentConfig, EmergencyPurchaseRule, EntityCount, GameState,
    PlantConfig, PlantId, StockCent,
};
use num_bigint::BigUint;
use thiserror::Error;

pub const CONTENT_VERSION: &str = "mvp-0.2.7";

pub const CLOVER: &str = "clover";
pub const SUNFLOWER: &str = "sunflower";
pub const BAMBOO: &str = "bamboo";
pub const CORN: &str = "corn";
pub const APPLE: &str = "apple";

pub const RABBIT: &str = "rabbit";
pub const HAMSTER: &str = "hamster";
pub const RED_PANDA: &str = "red-panda";
pub const CAPYBARA: &str = "capybara";
pub const DEER: &str = "deer";

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum MvpContentError {
    #[error(transparent)]
    InvalidCatalog(#[from] clickclackfarm_domain::ConfigError),
    #[error("MVP numerical invariant failed: {0}")]
    NumericalInvariant(&'static str),
    #[error(transparent)]
    InvalidInitialState(#[from] clickclackfarm_domain::StateError),
}

pub fn load_mvp_content() -> Result<ContentConfig, MvpContentError> {
    let ids = ids()?;
    let content = ContentConfig::try_new(
        CONTENT_VERSION,
        [
            plant(&ids.clover, &ids.rabbit, 0, 75, 500),
            plant(&ids.sunflower, &ids.hamster, 1, 300, 2_500),
            plant(&ids.bamboo, &ids.red_panda, 2, 1_200, 12_500),
            plant(&ids.corn, &ids.capybara, 3, 4_800, 62_500),
            plant(&ids.apple, &ids.deer, 4, 19_200, 312_500),
        ],
        [
            animal(&ids.rabbit, &ids.clover, 50, 25, 200),
            animal(&ids.hamster, &ids.sunflower, 200, 100, 1_000),
            animal(&ids.red_panda, &ids.bamboo, 800, 400, 5_000),
            animal(&ids.capybara, &ids.corn, 3_200, 1_600, 25_000),
            animal(&ids.deer, &ids.apple, 12_800, 6_400, 125_000),
        ],
        EmergencyPurchaseRule {
            animal_id: ids.rabbit,
            trigger_below_coins: 50_u64.into(),
        },
    )?;
    validate_mvp_numerical_invariants(&content)?;
    Ok(content)
}

pub fn new_mvp_game_state(content: &ContentConfig) -> Result<GameState, MvpContentError> {
    Ok(GameState::new(
        content,
        Coins::from(50_u64),
        [(PlantId::new(CLOVER)?, EntityCount::from(1_u64))],
        [(AnimalId::new(RABBIT)?, EntityCount::from(1_u64))],
    )?)
}

pub fn validate_mvp_numerical_invariants(content: &ContentConfig) -> Result<(), MvpContentError> {
    content.validate()?;
    if content.plants().len() != 5 || content.animals().len() != 5 {
        return Err(MvpContentError::NumericalInvariant(
            "exactly five plants and five animals are required",
        ));
    }

    let tier_ids = [
        (CLOVER, RABBIT),
        (SUNFLOWER, HAMSTER),
        (BAMBOO, RED_PANDA),
        (CORN, CAPYBARA),
        (APPLE, DEER),
    ];
    let expected_effective_costs = [100_u64, 400, 1_600, 6_400, 25_600];
    let mut previous_effective_cost: Option<BigUint> = None;
    let mut previous_hourly_growth: Option<BigUint> = None;

    for (index, ((plant_name, animal_name), expected_cost)) in
        tier_ids.iter().zip(expected_effective_costs).enumerate()
    {
        let plant_id = PlantId::new(*plant_name)?;
        let animal_id = AnimalId::new(*animal_name)?;
        let plant = content
            .plant(&plant_id)
            .ok_or(MvpContentError::NumericalInvariant("missing MVP plant"))?;
        let animal = content
            .animal(&animal_id)
            .ok_or(MvpContentError::NumericalInvariant("missing MVP animal"))?;

        if plant.fixed_land_slot != u8::try_from(index).expect("five tiers fit in u8")
            || plant.price_growth_num != BigUint::from(103_u64)
            || plant.price_growth_den != BigUint::from(100_u64)
            || animal.feeding_threshold != 12
        {
            return Err(MvpContentError::NumericalInvariant(
                "land slot, growth ratio, or feeding threshold drifted",
            ));
        }

        let effective_cost = animal.fixed_purchase_price_coins.as_biguint()
            + plant.base_price_coins.as_biguint()
            - animal.zero_growth_sell_price_coins.as_biguint();
        if effective_cost != BigUint::from(expected_cost) {
            return Err(MvpContentError::NumericalInvariant(
                "effective cost identity drifted",
            ));
        }
        let hourly_supply = plant.rate_cent_per_minute.as_biguint() * 60_u8;
        let hourly_growth = animal.bite_cent.as_biguint() * 150_u16;
        if hourly_supply != hourly_growth {
            return Err(MvpContentError::NumericalInvariant(
                "standard supply and demand are not 100 percent balanced",
            ));
        }

        if let (Some(previous_cost), Some(previous_growth)) =
            (&previous_effective_cost, &previous_hourly_growth)
            && (effective_cost != previous_cost * 4_u8 || hourly_growth != previous_growth * 5_u8)
        {
            return Err(MvpContentError::NumericalInvariant(
                "tier cost or income ladder drifted",
            ));
        }
        previous_effective_cost = Some(effective_cost);
        previous_hourly_growth = Some(hourly_growth);
    }
    Ok(())
}

fn plant(
    id: &PlantId,
    animal_id: &AnimalId,
    fixed_land_slot: u8,
    base_price_coins: u64,
    rate_cent_per_minute: u64,
) -> PlantConfig {
    PlantConfig {
        id: id.clone(),
        fixed_land_slot,
        base_price_coins: base_price_coins.into(),
        price_growth_num: BigUint::from(103_u64),
        price_growth_den: BigUint::from(100_u64),
        rate_cent_per_minute: StockCent::from(rate_cent_per_minute),
        paired_animal_id: animal_id.clone(),
    }
}

fn animal(
    id: &AnimalId,
    plant_id: &PlantId,
    purchase_price: u64,
    zero_growth_sale: u64,
    bite_cent: u64,
) -> AnimalConfig {
    AnimalConfig {
        id: id.clone(),
        fixed_purchase_price_coins: purchase_price.into(),
        zero_growth_sell_price_coins: zero_growth_sale.into(),
        feeding_threshold: 12,
        bite_cent: bite_cent.into(),
        food_plant_id: plant_id.clone(),
    }
}

struct Ids {
    clover: PlantId,
    sunflower: PlantId,
    bamboo: PlantId,
    corn: PlantId,
    apple: PlantId,
    rabbit: AnimalId,
    hamster: AnimalId,
    red_panda: AnimalId,
    capybara: AnimalId,
    deer: AnimalId,
}

fn ids() -> Result<Ids, clickclackfarm_domain::ConfigError> {
    Ok(Ids {
        clover: PlantId::new(CLOVER)?,
        sunflower: PlantId::new(SUNFLOWER)?,
        bamboo: PlantId::new(BAMBOO)?,
        corn: PlantId::new(CORN)?,
        apple: PlantId::new(APPLE)?,
        rabbit: AnimalId::new(RABBIT)?,
        hamster: AnimalId::new(HAMSTER)?,
        red_panda: AnimalId::new(RED_PANDA)?,
        capybara: AnimalId::new(CAPYBARA)?,
        deer: AnimalId::new(DEER)?,
    })
}

#[cfg(test)]
mod tests {
    use clickclackfarm_domain::{
        AnimalId, Coins, DomainEvent, EntityCount, GameState, PlantId, PurchaseKind,
        PurchasePricingBudget, PurchaseSelection, SaleSelection, apply_effective_inputs,
        apply_purchase_batch, apply_sale_batch, quote_purchase_batch,
        quote_purchase_batch_with_budget, settle_production, single_plant_price,
    };
    use num_bigint::BigUint;

    use super::{CLOVER, CONTENT_VERSION, RABBIT, load_mvp_content, new_mvp_game_state};

    #[test]
    fn bundled_content_passes_all_static_invariants() {
        let content = load_mvp_content().unwrap();
        assert_eq!(content.content_version(), CONTENT_VERSION);
        assert_eq!(content.plants().len(), 5);
        assert_eq!(content.animals().len(), 5);
    }

    #[test]
    fn new_save_matches_the_confirmed_initial_assets() {
        let content = load_mvp_content().unwrap();
        let state = new_mvp_game_state(&content).unwrap();
        let clover = clickclackfarm_domain::PlantId::new(CLOVER).unwrap();
        let rabbit = clickclackfarm_domain::AnimalId::new(RABBIT).unwrap();
        assert_eq!(state.wallet().coins, Coins::from(50_u64));
        assert_eq!(state.plants()[&clover].count, EntityCount::from(1_u64));
        assert_eq!(state.animals()[&rabbit].count, EntityCount::from(1_u64));
        assert!(
            state
                .plants()
                .values()
                .all(|plant| plant.stock_cent.is_zero())
        );
        assert!(
            state
                .animals()
                .values()
                .all(|animal| animal.total_growth_cent.is_zero()
                    && animal.feeding_progress.value() == 0
                    && animal.lifetime_paid_purchase_count.is_zero())
        );
        assert!(state.collection().is_plant_discovered(&clover));
        assert!(state.collection().is_animal_discovered(&rabbit));
    }

    #[test]
    fn all_reference_plant_prices_match_the_numeric_spec() {
        let content = load_mvp_content().unwrap();
        let clover = clickclackfarm_domain::PlantId::new(CLOVER).unwrap();
        let config = content.plant(&clover).unwrap();
        let cases = [
            (1_u64, 75),
            (5, 85),
            (10, 98),
            (20, 132),
            (30, 177),
            (50, 320),
        ];
        for (number, expected) in cases {
            assert_eq!(
                single_plant_price(config, &BigUint::from(number)).unwrap(),
                Coins::from(expected)
            );
        }
    }

    #[test]
    fn paid_purchase_increments_history_but_initial_grant_does_not() {
        let content = load_mvp_content().unwrap();
        let state = new_mvp_game_state(&content).unwrap();
        let rabbit = clickclackfarm_domain::AnimalId::new(RABBIT).unwrap();
        assert!(
            state.animals()[&rabbit]
                .lifetime_paid_purchase_count
                .is_zero()
        );
        let rich_state = clickclackfarm_domain::GameState::new(
            &content,
            Coins::from(1_000_u64),
            [],
            [(rabbit.clone(), EntityCount::from(1_u64))],
        )
        .unwrap();
        let result = apply_purchase_batch(
            &rich_state,
            &content,
            &[PurchaseSelection {
                kind: PurchaseKind::Animal(rabbit.clone()),
                quantity: 7_u64.into(),
            }],
        )
        .unwrap();
        assert_eq!(
            result.state.animals()[&rabbit]
                .lifetime_paid_purchase_count
                .as_biguint(),
            &BigUint::from(7_u64)
        );
    }

    #[test]
    fn five_tier_unified_purchase_feed_and_cross_species_sale_close_the_loop() {
        let content = load_mvp_content().unwrap();
        let mut selections = Vec::new();
        for id in [
            CLOVER,
            super::SUNFLOWER,
            super::BAMBOO,
            super::CORN,
            super::APPLE,
        ] {
            selections.push(PurchaseSelection {
                kind: PurchaseKind::Plant(PlantId::new(id).unwrap()),
                quantity: 1_u64.into(),
            });
        }
        for id in [
            RABBIT,
            super::HAMSTER,
            super::RED_PANDA,
            super::CAPYBARA,
            super::DEER,
        ] {
            selections.push(PurchaseSelection {
                kind: PurchaseKind::Animal(AnimalId::new(id).unwrap()),
                quantity: 1_u64.into(),
            });
        }
        let empty = GameState::new(&content, Coins::from(1_000_000_u64), [], []).unwrap();
        let purchased = apply_purchase_batch(&empty, &content, &selections).unwrap();
        assert_eq!(purchased.outcome.quote.lines.len(), 10);
        assert!(matches!(
            purchased.events.as_slice(),
            [
                DomainEvent::PurchaseBatchCommitted(_),
                DomainEvent::CollectionDiscovered { .. }
            ]
        ));
        assert!(
            purchased
                .state
                .animals()
                .values()
                .all(|animal| animal.lifetime_paid_purchase_count == 1_u64.into())
        );

        let produced =
            settle_production(&purchased.state, &content, &BigUint::from(24_u64)).unwrap();
        assert!(matches!(
            produced.events.as_slice(),
            [DomainEvent::ProductionSettled(_)]
        ));
        let fed =
            apply_effective_inputs(&produced.state, &content, &BigUint::from(12_u64)).unwrap();
        assert!(matches!(
            fed.events.as_slice(),
            [DomainEvent::FeedingAttempted(_)]
        ));
        assert!(
            fed.outcome
                .feeding
                .iter()
                .all(|line| line.successful_attempts == BigUint::from(1_u64)
                    && line.failed_attempts == BigUint::default()
                    && line.consumed_cent.as_biguint() == line.growth_added_cent.as_biguint())
        );
        assert!(
            fed.state
                .plants()
                .values()
                .all(|plant| plant.stock_cent.is_zero())
        );

        let sale_selections = content
            .animals()
            .keys()
            .cloned()
            .map(|animal_id| SaleSelection {
                animal_id,
                quantity: 1_u64.into(),
            })
            .collect::<Vec<_>>();
        let sold = apply_sale_batch(&fed.state, &content, &sale_selections).unwrap();
        assert!(matches!(
            sold.events.as_slice(),
            [DomainEvent::AnimalsSoldBatch(_)]
        ));
        assert_eq!(sold.outcome.quote.lines.len(), 5);
        assert!(sold.state.animals().values().all(|animal| {
            animal.count.is_zero()
                && animal.total_growth_cent.is_zero()
                && animal.feeding_progress.value() == 0
        }));
        assert_eq!(sold.state.collection().discovered_plants().len(), 5);
        assert_eq!(sold.state.collection().discovered_animals().len(), 5);
    }

    #[test]
    fn emergency_rabbit_immediate_sale_reaches_normal_price_in_at_most_two_claims() {
        let content = load_mvp_content().unwrap();
        let rabbit = AnimalId::new(RABBIT).unwrap();
        for starting_coins in 0_u64..50 {
            let mut state = GameState::new(&content, Coins::from(starting_coins), [], []).unwrap();
            let mut claims = 0_u8;
            while state.wallet().coins < Coins::from(50_u64) {
                claims += 1;
                assert!(claims <= 2);
                state = apply_purchase_batch(
                    &state,
                    &content,
                    &[PurchaseSelection {
                        kind: PurchaseKind::Animal(rabbit.clone()),
                        quantity: 1_u64.into(),
                    }],
                )
                .unwrap()
                .state;
                assert!(
                    state.animals()[&rabbit]
                        .lifetime_paid_purchase_count
                        .is_zero()
                );
                state = apply_sale_batch(
                    &state,
                    &content,
                    &[SaleSelection {
                        animal_id: rabbit.clone(),
                        quantity: 1_u64.into(),
                    }],
                )
                .unwrap()
                .state;
            }
            let normal_quote = quote_purchase_batch(
                &state,
                &content,
                &[PurchaseSelection {
                    kind: PurchaseKind::Animal(rabbit.clone()),
                    quantity: 1_u64.into(),
                }],
            )
            .unwrap();
            assert_eq!(normal_quote.total_cost, Coins::from(50_u64));
            assert!(!normal_quote.lines[0].emergency_free);
        }
    }

    #[test]
    fn five_species_share_one_plant_purchase_budget() {
        let content = load_mvp_content().unwrap();
        let state = GameState::new(&content, Coins::from(1_000_000_u64), [], []).unwrap();
        let selections = [
            CLOVER,
            super::SUNFLOWER,
            super::BAMBOO,
            super::CORN,
            super::APPLE,
        ]
        .into_iter()
        .map(|id| PurchaseSelection {
            kind: PurchaseKind::Plant(PlantId::new(id).unwrap()),
            quantity: 1_u64.into(),
        })
        .collect::<Vec<_>>();

        let result = quote_purchase_batch_with_budget(
            &state,
            &content,
            &selections,
            PurchasePricingBudget::new(4, 100, 100, 10_000),
        );
        assert!(matches!(
            result,
            Err(clickclackfarm_domain::DomainError::PlantPricingTotalUnitsBudgetExceeded {
                requested,
                limit: 4,
            }) if requested == "5"
        ));
    }
}
