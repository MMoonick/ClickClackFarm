use std::collections::BTreeSet;

use num_bigint::BigUint;
use num_traits::{ToPrimitive, Zero};

use crate::production::{plant_batch_cost, preflight_plant_sequence};
use crate::{
    AnimalId, Coins, ContentConfig, DomainError, DomainEvent, EntityCount, GameState, PlantId,
    Transition,
};

/// Explicit per-command budget for the exact, per-plant ceiling sum.
///
/// This is an engineering denial-of-service bound, not an economic holding cap.
/// WP3 may expose a smaller input limit, but must never replace exact pricing
/// with floating point or partial execution.
pub const DEFAULT_MAX_PLANT_UNITS_PER_BATCH: u64 = 100_000;
pub const DEFAULT_MAX_PLANT_SEQUENCE_NUMBER: u64 = 1_000_000;
pub const DEFAULT_MAX_ESTIMATED_PRICE_DIGITS: u64 = 20_000;
pub const DEFAULT_MAX_ESTIMATED_DIGIT_WORK: u64 = 250_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PurchasePricingBudget {
    max_total_plant_units: u64,
    max_plant_sequence_number: u64,
    max_estimated_price_digits: u64,
    max_estimated_digit_work: u64,
}

impl PurchasePricingBudget {
    #[must_use]
    pub const fn new(
        max_total_plant_units: u64,
        max_plant_sequence_number: u64,
        max_estimated_price_digits: u64,
        max_estimated_digit_work: u64,
    ) -> Self {
        Self {
            max_total_plant_units,
            max_plant_sequence_number,
            max_estimated_price_digits,
            max_estimated_digit_work,
        }
    }

    #[must_use]
    pub const fn max_total_plant_units(self) -> u64 {
        self.max_total_plant_units
    }

    #[must_use]
    pub const fn max_plant_sequence_number(self) -> u64 {
        self.max_plant_sequence_number
    }

    #[must_use]
    pub const fn max_estimated_price_digits(self) -> u64 {
        self.max_estimated_price_digits
    }

    #[must_use]
    pub const fn max_estimated_digit_work(self) -> u64 {
        self.max_estimated_digit_work
    }
}

impl Default for PurchasePricingBudget {
    fn default() -> Self {
        Self::new(
            DEFAULT_MAX_PLANT_UNITS_PER_BATCH,
            DEFAULT_MAX_PLANT_SEQUENCE_NUMBER,
            DEFAULT_MAX_ESTIMATED_PRICE_DIGITS,
            DEFAULT_MAX_ESTIMATED_DIGIT_WORK,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PurchaseKind {
    Plant(PlantId),
    Animal(AnimalId),
}

impl PurchaseKind {
    fn key(&self) -> String {
        match self {
            Self::Plant(id) => format!("plant:{id}"),
            Self::Animal(id) => format!("animal:{id}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PurchaseSelection {
    pub kind: PurchaseKind,
    pub quantity: EntityCount,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PurchaseLine {
    pub kind: PurchaseKind,
    pub quantity: EntityCount,
    pub cost: Coins,
    pub emergency_free: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PurchaseQuote {
    pub lines: Vec<PurchaseLine>,
    pub total_cost: Coins,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PurchaseReceipt {
    pub quote: PurchaseQuote,
    pub discovered_plants: Vec<PlantId>,
    pub discovered_animals: Vec<AnimalId>,
}

pub fn quote_purchase_batch(
    state: &GameState,
    content: &ContentConfig,
    selections: &[PurchaseSelection],
) -> Result<PurchaseQuote, DomainError> {
    quote_purchase_batch_with_budget(state, content, selections, PurchasePricingBudget::default())
}

pub fn quote_purchase_batch_with_budget(
    state: &GameState,
    content: &ContentConfig,
    selections: &[PurchaseSelection],
    budget: PurchasePricingBudget,
) -> Result<PurchaseQuote, DomainError> {
    state.validate(content)?;
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::new();
    for selection in selections {
        if !seen.insert(selection.kind.clone()) {
            return Err(DomainError::DuplicateSelection(selection.kind.key()));
        }
        if !selection.quantity.is_zero() {
            normalized.push(selection.clone());
        }
    }
    if normalized.is_empty() {
        return Err(DomainError::EmptySelection);
    }
    normalized.sort_by(|left, right| left.kind.cmp(&right.kind));
    preflight_purchase_pricing(state, content, &normalized, budget)?;

    let emergency_active = state.total_animal_count().is_zero()
        && state.wallet().coins.as_biguint()
            < content
                .emergency_purchase()
                .trigger_below_coins
                .as_biguint();
    let mut total_cost = BigUint::zero();
    let mut lines = Vec::with_capacity(normalized.len());

    for selection in normalized {
        let (cost, emergency_free) = match &selection.kind {
            PurchaseKind::Plant(id) => {
                let config = content
                    .plant(id)
                    .ok_or_else(|| DomainError::UnknownPlant(id.to_string()))?;
                let current = &state
                    .plants()
                    .get(id)
                    .ok_or_else(|| DomainError::UnknownPlant(id.to_string()))?
                    .count;
                (
                    plant_batch_cost(
                        config,
                        current.as_biguint(),
                        selection
                            .quantity
                            .as_biguint()
                            .to_u64()
                            .expect("command-wide pricing preflight bounded quantity"),
                    ),
                    false,
                )
            }
            PurchaseKind::Animal(id) => {
                let config = content
                    .animal(id)
                    .ok_or_else(|| DomainError::UnknownAnimal(id.to_string()))?;
                let is_emergency =
                    emergency_active && id == &content.emergency_purchase().animal_id;
                if is_emergency {
                    if selection.quantity != EntityCount::from(1_u64) {
                        return Err(DomainError::InvalidEmergencyQuantity);
                    }
                    (Coins::zero(), true)
                } else {
                    (
                        Coins::from_biguint(
                            selection.quantity.as_biguint()
                                * config.fixed_purchase_price_coins.as_biguint(),
                        ),
                        false,
                    )
                }
            }
        };
        total_cost += cost.as_biguint();
        lines.push(PurchaseLine {
            kind: selection.kind,
            quantity: selection.quantity,
            cost,
            emergency_free,
        });
    }

    Ok(PurchaseQuote {
        lines,
        total_cost: Coins::from_biguint(total_cost),
    })
}

pub fn apply_purchase_batch(
    state: &GameState,
    content: &ContentConfig,
    selections: &[PurchaseSelection],
) -> Result<Transition<PurchaseReceipt>, DomainError> {
    apply_purchase_batch_with_budget(state, content, selections, PurchasePricingBudget::default())
}

pub fn apply_purchase_batch_with_budget(
    state: &GameState,
    content: &ContentConfig,
    selections: &[PurchaseSelection],
    budget: PurchasePricingBudget,
) -> Result<Transition<PurchaseReceipt>, DomainError> {
    let quote = quote_purchase_batch_with_budget(state, content, selections, budget)?;
    if state.wallet().coins.as_biguint() < quote.total_cost.as_biguint() {
        return Err(DomainError::InsufficientCoins);
    }

    let mut next = state.clone();
    next.wallet_mut().coins =
        Coins::from_biguint(next.wallet().coins.as_biguint() - quote.total_cost.as_biguint());
    let mut discovered_plants = Vec::new();
    let mut discovered_animals = Vec::new();

    for line in &quote.lines {
        match &line.kind {
            PurchaseKind::Plant(id) => {
                let plant = next
                    .plants_mut()
                    .get_mut(id)
                    .expect("quote validated plant id");
                plant.count = EntityCount::from_biguint(
                    plant.count.as_biguint() + line.quantity.as_biguint(),
                );
                if next.collection_mut().discover_plant(id.clone()) {
                    discovered_plants.push(id.clone());
                }
            }
            PurchaseKind::Animal(id) => {
                let animal = next
                    .animals_mut()
                    .get_mut(id)
                    .expect("quote validated animal id");
                animal.count = EntityCount::from_biguint(
                    animal.count.as_biguint() + line.quantity.as_biguint(),
                );
                if !line.emergency_free {
                    animal.lifetime_paid_purchase_count =
                        crate::LifetimePurchaseCount::from_biguint(
                            animal.lifetime_paid_purchase_count.as_biguint()
                                + line.quantity.as_biguint(),
                        );
                }
                if next.collection_mut().discover_animal(id.clone()) {
                    discovered_animals.push(id.clone());
                }
            }
        }
    }
    next.validate(content)?;

    let mut events = vec![DomainEvent::PurchaseBatchCommitted(quote.clone())];
    if !discovered_plants.is_empty() || !discovered_animals.is_empty() {
        events.push(DomainEvent::CollectionDiscovered {
            plants: discovered_plants.clone(),
            animals: discovered_animals.clone(),
        });
    }
    Ok(Transition {
        state: next,
        outcome: PurchaseReceipt {
            quote,
            discovered_plants,
            discovered_animals,
        },
        events,
    })
}

fn preflight_purchase_pricing(
    state: &GameState,
    content: &ContentConfig,
    selections: &[PurchaseSelection],
    budget: PurchasePricingBudget,
) -> Result<(), DomainError> {
    let mut total_units = BigUint::zero();
    let mut total_work = 0_u64;
    for selection in selections {
        let PurchaseKind::Plant(id) = &selection.kind else {
            continue;
        };
        let config = content
            .plant(id)
            .ok_or_else(|| DomainError::UnknownPlant(id.to_string()))?;
        let current = state
            .plants()
            .get(id)
            .ok_or_else(|| DomainError::UnknownPlant(id.to_string()))?
            .count
            .as_biguint();

        total_units += selection.quantity.as_biguint();
        if total_units > BigUint::from(budget.max_total_plant_units()) {
            return Err(DomainError::PlantPricingTotalUnitsBudgetExceeded {
                requested: total_units.to_str_radix(10),
                limit: budget.max_total_plant_units(),
            });
        }
        let quantity = selection
            .quantity
            .as_biguint()
            .to_u64()
            .expect("total plant quantity was bounded by a u64 budget");
        let last_sequence = current + selection.quantity.as_biguint();
        let estimated_digits = preflight_plant_sequence(config, &last_sequence, budget)?;
        let line_work = quantity.saturating_mul(estimated_digits);
        total_work = total_work.saturating_add(line_work);
        if total_work > budget.max_estimated_digit_work() {
            return Err(DomainError::PlantPricingWorkBudgetExceeded {
                estimated: total_work,
                limit: budget.max_estimated_digit_work(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use num_bigint::BigUint;

    use crate::{
        AnimalConfig, AnimalId, Coins, ContentConfig, EmergencyPurchaseRule, EntityCount,
        GameState, PlantConfig, PlantId, StockCent,
    };

    use super::{
        PurchaseKind, PurchasePricingBudget, PurchaseSelection, apply_purchase_batch,
        apply_purchase_batch_with_budget, quote_purchase_batch, quote_purchase_batch_with_budget,
    };

    fn fixture(coins: u64, rabbits: u64) -> (ContentConfig, GameState, PlantId, AnimalId) {
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
                rate_cent_per_minute: StockCent::from(500_u64),
                paired_animal_id: animal_id.clone(),
            }],
            [AnimalConfig {
                id: animal_id.clone(),
                fixed_purchase_price_coins: 50_u64.into(),
                zero_growth_sell_price_coins: 25_u64.into(),
                feeding_threshold: 12,
                bite_cent: StockCent::from(200_u64),
                food_plant_id: plant_id.clone(),
            }],
            EmergencyPurchaseRule {
                animal_id: animal_id.clone(),
                trigger_below_coins: 50_u64.into(),
            },
        )
        .unwrap();
        let state = GameState::new(
            &content,
            Coins::from(coins),
            [],
            [(animal_id.clone(), EntityCount::from(rabbits))],
        )
        .unwrap();
        (content, state, plant_id, animal_id)
    }

    #[test]
    fn mixed_purchase_is_quoted_and_applied_as_one_result() {
        let (content, state, plant_id, animal_id) = fixture(1_000, 1);
        let selections = [
            PurchaseSelection {
                kind: PurchaseKind::Plant(plant_id.clone()),
                quantity: 2_u64.into(),
            },
            PurchaseSelection {
                kind: PurchaseKind::Animal(animal_id.clone()),
                quantity: 3_u64.into(),
            },
        ];
        let result = apply_purchase_batch(&state, &content, &selections).unwrap();
        assert_eq!(result.outcome.quote.total_cost, Coins::from(303_u64));
        assert_eq!(result.state.wallet().coins, Coins::from(697_u64));
        assert_eq!(result.state.plants()[&plant_id].count, EntityCount::from(2));
        assert_eq!(
            result.state.animals()[&animal_id].count,
            EntityCount::from(4)
        );
        assert_eq!(
            result.state.animals()[&animal_id]
                .lifetime_paid_purchase_count
                .as_biguint(),
            &BigUint::from(3_u64)
        );
    }

    #[test]
    fn insufficient_mixed_purchase_leaves_input_state_unchanged() {
        let (content, state, plant_id, animal_id) = fixture(100, 1);
        let selections = [
            PurchaseSelection {
                kind: PurchaseKind::Plant(plant_id),
                quantity: 1_u64.into(),
            },
            PurchaseSelection {
                kind: PurchaseKind::Animal(animal_id),
                quantity: 1_u64.into(),
            },
        ];
        assert!(apply_purchase_batch(&state, &content, &selections).is_err());
        assert_eq!(state.wallet().coins, Coins::from(100_u64));
    }

    #[test]
    fn emergency_rabbit_is_free_and_not_counted_as_paid() {
        let (content, state, _, animal_id) = fixture(49, 0);
        let selection = [PurchaseSelection {
            kind: PurchaseKind::Animal(animal_id.clone()),
            quantity: 1_u64.into(),
        }];
        let result = apply_purchase_batch(&state, &content, &selection).unwrap();
        assert!(result.outcome.quote.lines[0].emergency_free);
        assert_eq!(result.state.wallet().coins, Coins::from(49_u64));
        assert!(
            result.state.animals()[&animal_id]
                .lifetime_paid_purchase_count
                .is_zero()
        );
        assert!(result.state.collection().is_animal_discovered(&animal_id));
    }

    #[test]
    fn emergency_quantity_above_one_is_rejected() {
        let (content, state, _, animal_id) = fixture(0, 0);
        let result = quote_purchase_batch(
            &state,
            &content,
            &[PurchaseSelection {
                kind: PurchaseKind::Animal(animal_id),
                quantity: 2_u64.into(),
            }],
        );
        assert!(matches!(
            result,
            Err(crate::DomainError::InvalidEmergencyQuantity)
        ));
    }

    #[test]
    fn duplicate_and_zero_only_selections_are_rejected() {
        let (content, state, plant_id, _) = fixture(1_000, 1);
        let duplicate = [
            PurchaseSelection {
                kind: PurchaseKind::Plant(plant_id.clone()),
                quantity: 1_u64.into(),
            },
            PurchaseSelection {
                kind: PurchaseKind::Plant(plant_id.clone()),
                quantity: 2_u64.into(),
            },
        ];
        assert!(matches!(
            quote_purchase_batch(&state, &content, &duplicate),
            Err(crate::DomainError::DuplicateSelection(_))
        ));
        assert!(matches!(
            quote_purchase_batch(
                &state,
                &content,
                &[PurchaseSelection {
                    kind: PurchaseKind::Plant(plant_id),
                    quantity: EntityCount::zero(),
                }]
            ),
            Err(crate::DomainError::EmptySelection)
        ));
    }

    #[test]
    fn command_budget_accepts_limit_minus_one_and_limit_but_rejects_limit_plus_one() {
        let (content, state, plant_id, _) = fixture(10_000, 1);
        let budget = PurchasePricingBudget::new(10, 100, 100, 10_000);

        for quantity in [9_u64, 10] {
            let quote = quote_purchase_batch_with_budget(
                &state,
                &content,
                &[PurchaseSelection {
                    kind: PurchaseKind::Plant(plant_id.clone()),
                    quantity: quantity.into(),
                }],
                budget,
            );
            assert!(quote.is_ok(), "quantity {quantity} should fit the budget");
        }

        let result = apply_purchase_batch_with_budget(
            &state,
            &content,
            &[PurchaseSelection {
                kind: PurchaseKind::Plant(plant_id),
                quantity: 11_u64.into(),
            }],
            budget,
        );
        assert!(matches!(
            result,
            Err(crate::DomainError::PlantPricingTotalUnitsBudgetExceeded {
                requested,
                limit: 10,
            }) if requested == "11"
        ));
        assert_eq!(state.wallet().coins, Coins::from(10_000_u64));
        assert!(state.plants().values().all(|plant| plant.count.is_zero()));
    }

    #[test]
    fn enormous_current_count_fails_before_exact_power_calculation() {
        let (content, _, plant_id, animal_id) = fixture(10_000, 1);
        let enormous_count = BigUint::from(10_u8).pow(10_000);
        let state = GameState::new(
            &content,
            Coins::from(10_000_u64),
            [(plant_id.clone(), EntityCount::from_biguint(enormous_count))],
            [(animal_id, EntityCount::from(1_u64))],
        )
        .unwrap();
        let before = state.clone();

        let result = apply_purchase_batch(
            &state,
            &content,
            &[PurchaseSelection {
                kind: PurchaseKind::Plant(plant_id),
                quantity: 1_u64.into(),
            }],
        );

        assert!(matches!(
            result,
            Err(crate::DomainError::PlantPricingSequenceBudgetExceeded { .. })
        ));
        assert_eq!(state, before);
    }
}
