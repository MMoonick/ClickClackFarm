use num_bigint::BigUint;
use num_traits::{One, ToPrimitive, Zero};

use crate::purchase::PurchasePricingBudget;
use crate::{
    Coins, ContentConfig, DomainError, DomainEvent, GameState, PlantId, ProductionRemainder60,
    StockCent, Transition,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlantProduction {
    pub plant_id: PlantId,
    pub minted_cent: StockCent,
    pub resulting_remainder_60: ProductionRemainder60,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionSettlement {
    pub whole_seconds: BigUint,
    pub plants: Vec<PlantProduction>,
}

pub fn settle_production(
    state: &GameState,
    content: &ContentConfig,
    whole_seconds: &BigUint,
) -> Result<Transition<ProductionSettlement>, DomainError> {
    state.validate(content)?;
    let mut next = state.clone();
    let mut settlements = Vec::with_capacity(content.plants().len());

    for (id, config) in content.plants() {
        let plant = next
            .plants_mut()
            .get_mut(id)
            .expect("validated state contains every configured plant");
        let numerator = BigUint::from(plant.production_remainder_60.value())
            + plant.count.as_biguint() * config.rate_cent_per_minute.as_biguint() * whole_seconds;
        let minted = &numerator / 60_u8;
        let remainder = (&numerator % 60_u8)
            .to_u8()
            .expect("modulo 60 always fits in u8");
        plant.stock_cent = StockCent::from_biguint(plant.stock_cent.as_biguint() + &minted);
        plant.production_remainder_60 = ProductionRemainder60::try_new(remainder)
            .expect("modulo 60 always creates a valid production remainder");
        settlements.push(PlantProduction {
            plant_id: id.clone(),
            minted_cent: StockCent::from_biguint(minted),
            resulting_remainder_60: plant.production_remainder_60,
        });
    }

    let outcome = ProductionSettlement {
        whole_seconds: whole_seconds.clone(),
        plants: settlements,
    };
    Ok(Transition {
        state: next,
        events: vec![DomainEvent::ProductionSettled(outcome.clone())],
        outcome,
    })
}

pub fn single_plant_price(
    config: &crate::PlantConfig,
    plant_number: &BigUint,
) -> Result<Coins, DomainError> {
    single_plant_price_with_budget(config, plant_number, PurchasePricingBudget::default())
}

pub fn single_plant_price_with_budget(
    config: &crate::PlantConfig,
    plant_number: &BigUint,
    budget: PurchasePricingBudget,
) -> Result<Coins, DomainError> {
    if plant_number.is_zero() {
        return Err(DomainError::InvalidPlantSequenceNumber);
    }
    let estimated_digits = preflight_plant_sequence(config, plant_number, budget)?;
    if estimated_digits > budget.max_estimated_digit_work() {
        return Err(DomainError::PlantPricingWorkBudgetExceeded {
            estimated: estimated_digits,
            limit: budget.max_estimated_digit_work(),
        });
    }
    let exponent = plant_number - BigUint::one();
    let numerator =
        config.base_price_coins.as_biguint() * pow_biguint(&config.price_growth_num, &exponent);
    let denominator = pow_biguint(&config.price_growth_den, &exponent);
    Ok(Coins::from_biguint(ceil_div(&numerator, &denominator)))
}

pub(crate) fn plant_batch_cost(
    config: &crate::PlantConfig,
    current_count: &BigUint,
    quantity: u64,
) -> Coins {
    let mut numerator =
        config.base_price_coins.as_biguint() * pow_biguint(&config.price_growth_num, current_count);
    let mut denominator = pow_biguint(&config.price_growth_den, current_count);
    let mut total = BigUint::zero();
    for _ in 0..quantity {
        total += ceil_div(&numerator, &denominator);
        numerator *= &config.price_growth_num;
        denominator *= &config.price_growth_den;
    }
    Coins::from_biguint(total)
}

pub(crate) fn preflight_plant_sequence(
    config: &crate::PlantConfig,
    plant_number: &BigUint,
    budget: PurchasePricingBudget,
) -> Result<u64, DomainError> {
    if plant_number > &BigUint::from(budget.max_plant_sequence_number()) {
        return Err(DomainError::PlantPricingSequenceBudgetExceeded {
            plant_id: config.id.to_string(),
            requested: plant_number.to_str_radix(10),
            limit: budget.max_plant_sequence_number(),
        });
    }
    let sequence = plant_number
        .to_u64()
        .expect("sequence was bounded by a u64 pricing budget");
    let exponent = sequence.saturating_sub(1);
    // The exact calculation materializes numerator^exponent and
    // denominator^exponent independently. Decimal string lengths provide a
    // conservative, integer-only upper bound for those intermediate values
    // before either power is attempted.
    let growth_factor_digits = u64::try_from(
        config
            .price_growth_num
            .to_str_radix(10)
            .len()
            .max(config.price_growth_den.to_str_radix(10).len()),
    )
    .unwrap_or(u64::MAX);
    let growth_digits = exponent.saturating_mul(growth_factor_digits);
    let base_digits =
        u64::try_from(config.base_price_coins.to_decimal_string().len()).unwrap_or(u64::MAX);
    let estimated_digits = base_digits.saturating_add(growth_digits);
    if estimated_digits > budget.max_estimated_price_digits() {
        return Err(DomainError::PlantPricingDigitsBudgetExceeded {
            plant_id: config.id.to_string(),
            estimated: estimated_digits,
            limit: budget.max_estimated_price_digits(),
        });
    }
    Ok(estimated_digits)
}

fn ceil_div(numerator: &BigUint, denominator: &BigUint) -> BigUint {
    let quotient = numerator / denominator;
    if numerator % denominator == BigUint::zero() {
        quotient
    } else {
        quotient + BigUint::one()
    }
}

fn pow_biguint(base: &BigUint, exponent: &BigUint) -> BigUint {
    let mut result = BigUint::one();
    let mut factor = base.clone();
    let mut remaining = exponent.clone();
    while !remaining.is_zero() {
        if (&remaining & BigUint::one()) == BigUint::one() {
            result *= &factor;
        }
        remaining >>= 1_usize;
        if !remaining.is_zero() {
            factor = &factor * &factor;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use num_bigint::BigUint;

    use super::{settle_production, single_plant_price};
    use crate::{
        AnimalConfig, AnimalId, Coins, ContentConfig, EmergencyPurchaseRule, EntityCount,
        GameState, PlantConfig, PlantId, StockCent,
    };

    fn fixture() -> (ContentConfig, PlantId) {
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
                animal_id,
                trigger_below_coins: 50_u64.into(),
            },
        )
        .unwrap();
        (content, plant_id)
    }

    #[test]
    fn exact_reference_plant_prices_do_not_use_float() {
        let (content, plant_id) = fixture();
        let plant = content.plant(&plant_id).unwrap();
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
                single_plant_price(plant, &BigUint::from(number)).unwrap(),
                Coins::from(expected)
            );
        }
    }

    #[test]
    fn production_preserves_fractional_minute_remainder_across_chunks() {
        let (content, plant_id) = fixture();
        let state = GameState::new(
            &content,
            Coins::zero(),
            [(plant_id.clone(), EntityCount::from(7_u64))],
            [],
        )
        .unwrap();
        let whole = settle_production(&state, &content, &BigUint::from(3_601_u64)).unwrap();
        let first = settle_production(&state, &content, &BigUint::from(1_337_u64)).unwrap();
        let second = settle_production(&first.state, &content, &BigUint::from(2_264_u64)).unwrap();
        assert_eq!(
            whole.state.plants()[&plant_id].stock_cent,
            second.state.plants()[&plant_id].stock_cent
        );
        assert_eq!(
            whole.state.plants()[&plant_id].production_remainder_60,
            second.state.plants()[&plant_id].production_remainder_60
        );
    }

    #[test]
    fn production_supports_values_beyond_u128() {
        let (content, plant_id) = fixture();
        let huge = BigUint::from(10_u8).pow(100);
        let state = GameState::new(
            &content,
            Coins::zero(),
            [(plant_id.clone(), EntityCount::from_biguint(huge.clone()))],
            [],
        )
        .unwrap();
        let result = settle_production(&state, &content, &huge).unwrap();
        assert!(result.state.plants()[&plant_id].stock_cent > StockCent::zero());
    }
}
