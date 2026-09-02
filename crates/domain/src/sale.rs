use std::collections::BTreeSet;

use num_bigint::BigUint;
use num_traits::Zero;

use crate::{
    AnimalId, Coins, ContentConfig, DomainError, DomainEvent, EntityCount, FeedingProgress,
    GameState, GrowthCent, Transition,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaleSelection {
    pub animal_id: AnimalId,
    pub quantity: EntityCount,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaleLine {
    pub animal_id: AnimalId,
    pub quantity: EntityCount,
    pub sold_growth_cent: GrowthCent,
    pub coins: Coins,
    pub sells_entire_group: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaleQuote {
    pub lines: Vec<SaleLine>,
    pub total_coins: Coins,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaleReceipt {
    pub quote: SaleQuote,
}

pub fn quote_sale_batch(
    state: &GameState,
    content: &ContentConfig,
    selections: &[SaleSelection],
) -> Result<SaleQuote, DomainError> {
    state.validate(content)?;
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::new();
    for selection in selections {
        if !seen.insert(selection.animal_id.clone()) {
            return Err(DomainError::DuplicateSelection(format!(
                "animal:{}",
                selection.animal_id
            )));
        }
        if !selection.quantity.is_zero() {
            normalized.push(selection.clone());
        }
    }
    if normalized.is_empty() {
        return Err(DomainError::EmptySelection);
    }
    normalized.sort_by(|left, right| left.animal_id.cmp(&right.animal_id));

    let mut total_coins = BigUint::zero();
    let mut lines = Vec::with_capacity(normalized.len());
    for selection in normalized {
        let config = content
            .animal(&selection.animal_id)
            .ok_or_else(|| DomainError::UnknownAnimal(selection.animal_id.to_string()))?;
        let animal = state
            .animals()
            .get(&selection.animal_id)
            .ok_or_else(|| DomainError::UnknownAnimal(selection.animal_id.to_string()))?;
        if selection.quantity.as_biguint() > animal.count.as_biguint() {
            return Err(DomainError::SaleQuantityExceedsOwned(
                selection.animal_id.to_string(),
            ));
        }
        let sells_entire_group = selection.quantity == animal.count;
        let sold_growth = if sells_entire_group {
            animal.total_growth_cent.as_biguint().clone()
        } else {
            animal.total_growth_cent.as_biguint() * selection.quantity.as_biguint()
                / animal.count.as_biguint()
        };
        let coins = selection.quantity.as_biguint()
            * config.zero_growth_sell_price_coins.as_biguint()
            + &sold_growth / 100_u8;
        total_coins += &coins;
        lines.push(SaleLine {
            animal_id: selection.animal_id,
            quantity: selection.quantity,
            sold_growth_cent: GrowthCent::from_biguint(sold_growth),
            coins: Coins::from_biguint(coins),
            sells_entire_group,
        });
    }

    Ok(SaleQuote {
        lines,
        total_coins: Coins::from_biguint(total_coins),
    })
}

pub fn apply_sale_batch(
    state: &GameState,
    content: &ContentConfig,
    selections: &[SaleSelection],
) -> Result<Transition<SaleReceipt>, DomainError> {
    let quote = quote_sale_batch(state, content, selections)?;
    let mut next = state.clone();
    for line in &quote.lines {
        let animal = next
            .animals_mut()
            .get_mut(&line.animal_id)
            .expect("quote validated animal id");
        animal.count =
            EntityCount::from_biguint(animal.count.as_biguint() - line.quantity.as_biguint());
        animal.total_growth_cent = GrowthCent::from_biguint(
            animal.total_growth_cent.as_biguint() - line.sold_growth_cent.as_biguint(),
        );
        if line.sells_entire_group {
            animal.total_growth_cent = GrowthCent::zero();
            animal.feeding_progress = FeedingProgress::default();
        }
    }
    next.wallet_mut().coins =
        Coins::from_biguint(next.wallet().coins.as_biguint() + quote.total_coins.as_biguint());
    next.validate(content)?;
    Ok(Transition {
        state: next,
        events: vec![DomainEvent::AnimalsSoldBatch(quote.clone())],
        outcome: SaleReceipt { quote },
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use num_bigint::BigUint;
    use proptest::prelude::*;

    use crate::{
        AnimalConfig, AnimalId, AnimalState, Coins, ContentConfig, EmergencyPurchaseRule,
        EntityCount, FeedingProgress, GameState, GrowthCent, LifetimePurchaseCount, PlantConfig,
        PlantId, PlantState, ProductionRemainder60, SaleSelection, StockCent, Wallet,
    };

    use super::{apply_sale_batch, quote_sale_batch};

    fn fixture(
        animals: u64,
        growth_cent: BigUint,
        progress: u8,
    ) -> (ContentConfig, GameState, AnimalId) {
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
        let mut plants = BTreeMap::new();
        plants.insert(
            plant_id,
            PlantState {
                count: 1_u64.into(),
                stock_cent: StockCent::zero(),
                production_remainder_60: ProductionRemainder60::default(),
            },
        );
        let mut animal_states = BTreeMap::new();
        animal_states.insert(
            animal_id.clone(),
            AnimalState {
                count: animals.into(),
                total_growth_cent: GrowthCent::from_biguint(growth_cent),
                feeding_progress: FeedingProgress::new_unchecked(progress),
                lifetime_paid_purchase_count: LifetimePurchaseCount::zero(),
            },
        );
        let initial = GameState::new(
            &content,
            Coins::zero(),
            [(PlantId::new("clover").unwrap(), EntityCount::from(1_u64))],
            [(animal_id.clone(), EntityCount::from(animals))],
        )
        .unwrap();
        let state = GameState::try_from_parts(
            "test".to_owned(),
            Wallet {
                coins: Coins::zero(),
            },
            plants,
            animal_states,
            initial.collection().clone(),
            &content,
        )
        .unwrap();
        (content, state, animal_id)
    }

    #[test]
    fn partial_sale_conserves_growth_and_progress() {
        let (content, state, animal_id) = fixture(3, BigUint::from(100_u64), 7);
        let result = apply_sale_batch(
            &state,
            &content,
            &[SaleSelection {
                animal_id: animal_id.clone(),
                quantity: 1_u64.into(),
            }],
        )
        .unwrap();
        let line = &result.outcome.quote.lines[0];
        assert_eq!(line.sold_growth_cent, GrowthCent::from(33_u64));
        assert_eq!(
            result.state.animals()[&animal_id].total_growth_cent,
            GrowthCent::from(67_u64)
        );
        assert_eq!(
            result.state.animals()[&animal_id].feeding_progress.value(),
            7
        );
        assert_eq!(
            line.sold_growth_cent.as_biguint()
                + result.state.animals()[&animal_id]
                    .total_growth_cent
                    .as_biguint(),
            BigUint::from(100_u64)
        );
    }

    #[test]
    fn full_sale_takes_all_growth_and_clears_progress() {
        let (content, state, animal_id) = fixture(3, BigUint::from(101_u64), 11);
        let result = apply_sale_batch(
            &state,
            &content,
            &[SaleSelection {
                animal_id: animal_id.clone(),
                quantity: 3_u64.into(),
            }],
        )
        .unwrap();
        assert!(result.state.animals()[&animal_id].count.is_zero());
        assert!(
            result.state.animals()[&animal_id]
                .total_growth_cent
                .is_zero()
        );
        assert_eq!(
            result.state.animals()[&animal_id].feeding_progress.value(),
            0
        );
    }

    #[test]
    fn invalid_batch_is_rejected_before_any_species_can_change() {
        let (content, state, animal_id) = fixture(3, BigUint::from(100_u64), 0);
        let result = apply_sale_batch(
            &state,
            &content,
            &[SaleSelection {
                animal_id,
                quantity: 4_u64.into(),
            }],
        );
        assert!(result.is_err());
        assert_eq!(state.wallet().coins, Coins::zero());
    }

    #[test]
    fn partial_sale_supports_growth_far_beyond_u128() {
        let huge = BigUint::from(10_u8).pow(100);
        let (content, state, animal_id) = fixture(3, huge.clone(), 0);
        let result = apply_sale_batch(
            &state,
            &content,
            &[SaleSelection {
                animal_id: animal_id.clone(),
                quantity: 1_u64.into(),
            }],
        )
        .unwrap();
        assert_eq!(
            result.outcome.quote.lines[0].sold_growth_cent.as_biguint()
                + result.state.animals()[&animal_id]
                    .total_growth_cent
                    .as_biguint(),
            huge
        );
    }

    proptest! {
        #[test]
        fn split_sales_never_pay_more_than_combined(
            animals in 1_u16..50,
            growth_cent in 0_u64..1_000_000,
        ) {
            let (content, state, animal_id) = fixture(
                u64::from(animals),
                BigUint::from(growth_cent),
                0,
            );
            let combined = quote_sale_batch(
                &state,
                &content,
                &[SaleSelection {
                    animal_id: animal_id.clone(),
                    quantity: u64::from(animals).into(),
                }],
            ).unwrap();
            let mut current = state;
            let mut split_coins = BigUint::default();
            for _ in 0..animals {
                let sale = apply_sale_batch(
                    &current,
                    &content,
                    &[SaleSelection {
                        animal_id: animal_id.clone(),
                        quantity: 1_u64.into(),
                    }],
                ).unwrap();
                split_coins += sale.outcome.quote.total_coins.as_biguint();
                current = sale.state;
            }
            prop_assert!(split_coins <= *combined.total_coins.as_biguint());
            prop_assert!(current.animals()[&animal_id].total_growth_cent.is_zero());
        }
    }
}
