use num_bigint::BigUint;
use num_traits::{ToPrimitive, Zero};

use crate::{
    AnimalId, ContentConfig, DomainError, DomainEvent, FeedingProgress, GameState, GrowthCent,
    PlantId, StockCent, Transition,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeedingSettlement {
    pub animal_id: AnimalId,
    pub food_plant_id: PlantId,
    pub attempts: BigUint,
    pub successful_attempts: BigUint,
    pub failed_attempts: BigUint,
    pub consumed_cent: StockCent,
    pub growth_added_cent: GrowthCent,
    pub resulting_progress: FeedingProgress,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputSettlement {
    pub effective_input_count: BigUint,
    pub feeding: Vec<FeedingSettlement>,
}

/// Applies a count of effective inputs with no production boundary between them.
///
/// WP3 must settle production before each distinct observed input timestamp. It
/// may call this in bulk only for inputs that share a production boundary; the
/// result is exactly equivalent to applying those inputs one by one.
pub fn apply_effective_inputs(
    state: &GameState,
    content: &ContentConfig,
    count: &BigUint,
) -> Result<Transition<InputSettlement>, DomainError> {
    state.validate(content)?;
    let mut next = state.clone();
    let mut feeding = Vec::with_capacity(content.animals().len());

    for (animal_id, animal_config) in content.animals() {
        let current_animal = next
            .animals()
            .get(animal_id)
            .expect("validated state contains every configured animal");
        if current_animal.count.is_zero() {
            continue;
        }

        let threshold = BigUint::from(animal_config.feeding_threshold);
        let total_progress = BigUint::from(current_animal.feeding_progress.value()) + count;
        let attempts = &total_progress / &threshold;
        let resulting_progress = (&total_progress % &threshold)
            .to_u8()
            .expect("feeding threshold is bounded by u8");
        let consumption_per_attempt =
            current_animal.count.as_biguint() * animal_config.bite_cent.as_biguint();
        let food = next
            .plants()
            .get(&animal_config.food_plant_id)
            .expect("validated reciprocal content contains food plant");
        let affordable_attempts = if consumption_per_attempt.is_zero() {
            BigUint::zero()
        } else {
            food.stock_cent.as_biguint() / &consumption_per_attempt
        };
        let successful_attempts = attempts.clone().min(affordable_attempts);
        let failed_attempts = &attempts - &successful_attempts;
        let transferred_cent = &successful_attempts * &consumption_per_attempt;

        {
            let plant = next
                .plants_mut()
                .get_mut(&animal_config.food_plant_id)
                .expect("validated reciprocal content contains food plant");
            plant.stock_cent =
                StockCent::from_biguint(plant.stock_cent.as_biguint() - &transferred_cent);
        }
        {
            let animal = next
                .animals_mut()
                .get_mut(animal_id)
                .expect("validated state contains every configured animal");
            animal.total_growth_cent =
                GrowthCent::from_biguint(animal.total_growth_cent.as_biguint() + &transferred_cent);
            animal.feeding_progress = FeedingProgress::new_unchecked(resulting_progress);
        }

        feeding.push(FeedingSettlement {
            animal_id: animal_id.clone(),
            food_plant_id: animal_config.food_plant_id.clone(),
            attempts,
            successful_attempts,
            failed_attempts,
            consumed_cent: StockCent::from_biguint(transferred_cent.clone()),
            growth_added_cent: GrowthCent::from_biguint(transferred_cent),
            resulting_progress: FeedingProgress::new_unchecked(resulting_progress),
        });
    }

    let outcome = InputSettlement {
        effective_input_count: count.clone(),
        feeding,
    };
    Ok(Transition {
        state: next,
        events: vec![DomainEvent::FeedingAttempted(outcome.clone())],
        outcome,
    })
}

#[cfg(test)]
mod tests {
    use num_bigint::BigUint;
    use proptest::prelude::*;

    use crate::{
        AnimalConfig, AnimalId, Coins, ContentConfig, EmergencyPurchaseRule, EntityCount,
        GameState, PlantConfig, PlantId, PlantState, ProductionRemainder60, StockCent, Wallet,
    };

    use super::apply_effective_inputs;

    fn fixture(stock_cent: u64, animals: u64) -> (ContentConfig, GameState, PlantId, AnimalId) {
        let plant_id = PlantId::new("clover").unwrap();
        let animal_id = AnimalId::new("rabbit").unwrap();
        let content = ContentConfig::try_new(
            "test",
            [PlantConfig {
                id: plant_id.clone(),
                fixed_land_slot: 0,
                base_price_coins: 75_u64.into(),
                price_growth_num: BigUint::from(103_u64),
                price_growth_den: BigUint::from(100_u64),
                rate_cent_per_minute: 500_u64.into(),
                paired_animal_id: animal_id.clone(),
            }],
            [AnimalConfig {
                id: animal_id.clone(),
                fixed_purchase_price_coins: 50_u64.into(),
                zero_growth_sell_price_coins: 25_u64.into(),
                feeding_threshold: 12,
                bite_cent: 200_u64.into(),
                food_plant_id: plant_id.clone(),
            }],
            EmergencyPurchaseRule {
                animal_id: animal_id.clone(),
                trigger_below_coins: 50_u64.into(),
            },
        )
        .unwrap();
        let initial = GameState::new(
            &content,
            Coins::zero(),
            [(plant_id.clone(), EntityCount::from(1_u64))],
            [(animal_id.clone(), EntityCount::from(animals))],
        )
        .unwrap();
        let mut plants = initial.plants().clone();
        plants.insert(
            plant_id.clone(),
            PlantState {
                count: 1_u64.into(),
                stock_cent: stock_cent.into(),
                production_remainder_60: ProductionRemainder60::default(),
            },
        );
        let state = GameState::try_from_parts(
            "test".to_owned(),
            Wallet {
                coins: Coins::zero(),
            },
            plants,
            initial.animals().clone(),
            initial.collection().clone(),
            &content,
        )
        .unwrap();
        (content, state, plant_id, animal_id)
    }

    #[test]
    fn threshold_boundaries_are_n_minus_one_n_and_n_plus_one() {
        for (inputs, attempts, progress) in [(11_u64, 0_u64, 11), (12, 1, 0), (13, 1, 1)] {
            let (content, state, _, animal_id) = fixture(10_000, 1);
            let result = apply_effective_inputs(&state, &content, &BigUint::from(inputs)).unwrap();
            let line = result
                .outcome
                .feeding
                .iter()
                .find(|line| line.animal_id == animal_id)
                .unwrap();
            assert_eq!(line.attempts, BigUint::from(attempts));
            assert_eq!(line.resulting_progress.value(), progress);
        }
    }

    #[test]
    fn insufficient_food_fails_the_whole_group_and_resets_progress() {
        let (content, state, plant_id, animal_id) = fixture(399, 2);
        let result = apply_effective_inputs(&state, &content, &BigUint::from(12_u64)).unwrap();
        assert_eq!(
            result.state.plants()[&plant_id].stock_cent,
            StockCent::from(399)
        );
        assert!(
            result.state.animals()[&animal_id]
                .total_growth_cent
                .is_zero()
        );
        assert_eq!(
            result.state.animals()[&animal_id].feeding_progress.value(),
            0
        );
        assert_eq!(
            result.outcome.feeding[0].failed_attempts,
            BigUint::from(1_u64)
        );
    }

    #[test]
    fn successful_feeding_transfers_stock_to_growth_one_for_one() {
        let (content, state, plant_id, animal_id) = fixture(1_000, 2);
        let result = apply_effective_inputs(&state, &content, &BigUint::from(12_u64)).unwrap();
        assert_eq!(
            result.state.plants()[&plant_id].stock_cent,
            StockCent::from(600)
        );
        assert_eq!(
            result.state.animals()[&animal_id]
                .total_growth_cent
                .as_biguint(),
            &BigUint::from(400_u64)
        );
    }

    proptest! {
        #[test]
        fn bulk_input_matches_repeated_single_input(inputs in 0_u16..500, stock in 0_u64..100_000) {
            let (content, initial, _, _) = fixture(stock, 3);
            let bulk = apply_effective_inputs(&initial, &content, &BigUint::from(inputs)).unwrap();
            let mut repeated = initial;
            for _ in 0..inputs {
                repeated = apply_effective_inputs(&repeated, &content, &BigUint::from(1_u8)).unwrap().state;
            }
            prop_assert_eq!(bulk.state, repeated);
        }
    }
}
