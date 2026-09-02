use std::collections::{BTreeMap, BTreeSet};

use num_bigint::BigUint;
use num_traits::Zero;

use crate::{
    AnimalId, Coins, ContentConfig, DomainEvent, EntityCount, GrowthCent, LifetimePurchaseCount,
    PlantId, StateError, StockCent,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FeedingProgress(u8);

impl FeedingProgress {
    #[must_use]
    pub const fn new_unchecked(value: u8) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn value(self) -> u8 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProductionRemainder60(u8);

impl ProductionRemainder60 {
    pub fn try_new(value: u8) -> Result<Self, StateError> {
        if value < 60 {
            Ok(Self(value))
        } else {
            Err(StateError::InvalidProductionRemainder)
        }
    }

    #[must_use]
    pub const fn value(self) -> u8 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wallet {
    pub coins: Coins,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlantState {
    pub count: EntityCount,
    pub stock_cent: StockCent,
    pub production_remainder_60: ProductionRemainder60,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnimalState {
    pub count: EntityCount,
    pub total_growth_cent: GrowthCent,
    pub feeding_progress: FeedingProgress,
    pub lifetime_paid_purchase_count: LifetimePurchaseCount,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CollectionState {
    discovered_plants: BTreeSet<PlantId>,
    discovered_animals: BTreeSet<AnimalId>,
}

impl CollectionState {
    #[must_use]
    pub fn from_discovered(
        discovered_plants: impl IntoIterator<Item = PlantId>,
        discovered_animals: impl IntoIterator<Item = AnimalId>,
    ) -> Self {
        Self {
            discovered_plants: discovered_plants.into_iter().collect(),
            discovered_animals: discovered_animals.into_iter().collect(),
        }
    }

    #[must_use]
    pub fn discovered_plants(&self) -> &BTreeSet<PlantId> {
        &self.discovered_plants
    }

    #[must_use]
    pub fn discovered_animals(&self) -> &BTreeSet<AnimalId> {
        &self.discovered_animals
    }

    #[must_use]
    pub fn is_plant_discovered(&self, id: &PlantId) -> bool {
        self.discovered_plants.contains(id)
    }

    #[must_use]
    pub fn is_animal_discovered(&self, id: &AnimalId) -> bool {
        self.discovered_animals.contains(id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameState {
    content_version: String,
    wallet: Wallet,
    plants: BTreeMap<PlantId, PlantState>,
    animals: BTreeMap<AnimalId, AnimalState>,
    collection: CollectionState,
}

impl GameState {
    pub fn new(
        content: &ContentConfig,
        coins: Coins,
        initial_plants: impl IntoIterator<Item = (PlantId, EntityCount)>,
        initial_animals: impl IntoIterator<Item = (AnimalId, EntityCount)>,
    ) -> Result<Self, StateError> {
        let mut plants = content
            .plants()
            .keys()
            .cloned()
            .map(|id| {
                (
                    id,
                    PlantState {
                        count: EntityCount::zero(),
                        stock_cent: StockCent::zero(),
                        production_remainder_60: ProductionRemainder60::default(),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut animals = content
            .animals()
            .keys()
            .cloned()
            .map(|id| {
                (
                    id,
                    AnimalState {
                        count: EntityCount::zero(),
                        total_growth_cent: GrowthCent::zero(),
                        feeding_progress: FeedingProgress::default(),
                        lifetime_paid_purchase_count: LifetimePurchaseCount::zero(),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut collection = CollectionState::default();

        let mut seen_plants = BTreeSet::new();
        for (id, count) in initial_plants {
            if !seen_plants.insert(id.clone()) {
                return Err(StateError::DuplicateInitialGrant(id.to_string()));
            }
            let plant = plants
                .get_mut(&id)
                .ok_or_else(|| StateError::UnknownPlant(id.to_string()))?;
            plant.count = count;
            if !plant.count.is_zero() {
                collection.discovered_plants.insert(id);
            }
        }

        let mut seen_animals = BTreeSet::new();
        for (id, count) in initial_animals {
            if !seen_animals.insert(id.clone()) {
                return Err(StateError::DuplicateInitialGrant(id.to_string()));
            }
            let animal = animals
                .get_mut(&id)
                .ok_or_else(|| StateError::UnknownAnimal(id.to_string()))?;
            animal.count = count;
            if !animal.count.is_zero() {
                collection.discovered_animals.insert(id);
            }
        }

        let state = Self {
            content_version: content.content_version().to_owned(),
            wallet: Wallet { coins },
            plants,
            animals,
            collection,
        };
        state.validate(content)?;
        Ok(state)
    }

    pub fn try_from_parts(
        content_version: String,
        wallet: Wallet,
        plants: BTreeMap<PlantId, PlantState>,
        animals: BTreeMap<AnimalId, AnimalState>,
        collection: CollectionState,
        content: &ContentConfig,
    ) -> Result<Self, StateError> {
        let state = Self {
            content_version,
            wallet,
            plants,
            animals,
            collection,
        };
        state.validate(content)?;
        Ok(state)
    }

    pub fn validate(&self, content: &ContentConfig) -> Result<(), StateError> {
        if self.content_version != content.content_version() {
            return Err(StateError::ContentVersionMismatch {
                state: self.content_version.clone(),
                catalog: content.content_version().to_owned(),
            });
        }
        if self.plants.keys().ne(content.plants().keys())
            || self.animals.keys().ne(content.animals().keys())
        {
            return Err(StateError::SpeciesSetMismatch);
        }
        for (id, plant) in &self.plants {
            if plant.production_remainder_60.value() >= 60 {
                return Err(StateError::InvalidProductionRemainder);
            }
            if content.plant(id).is_none() {
                return Err(StateError::UnknownPlant(id.to_string()));
            }
            if plant.count.is_zero() && plant.production_remainder_60.value() != 0 {
                return Err(StateError::EmptyPlantHasProductionRemainder(id.to_string()));
            }
            if plant.count.is_zero() && !plant.stock_cent.is_zero() {
                return Err(StateError::EmptyPlantHasStock(id.to_string()));
            }
            if !plant.count.is_zero() && !self.collection.discovered_plants.contains(id) {
                return Err(StateError::OwnedSpeciesUndiscovered(id.to_string()));
            }
        }
        for (id, animal) in &self.animals {
            let config = content
                .animal(id)
                .ok_or_else(|| StateError::UnknownAnimal(id.to_string()))?;
            if animal.feeding_progress.value() >= config.feeding_threshold {
                return Err(StateError::InvalidFeedingProgress {
                    animal_id: id.to_string(),
                    threshold: config.feeding_threshold,
                });
            }
            if animal.count.is_zero()
                && (!animal.total_growth_cent.is_zero() || animal.feeding_progress.value() != 0)
            {
                return Err(StateError::EmptyAnimalHasProgress(id.to_string()));
            }
            if (!animal.count.is_zero() || !animal.lifetime_paid_purchase_count.is_zero())
                && !self.collection.discovered_animals.contains(id)
            {
                return Err(StateError::OwnedSpeciesUndiscovered(id.to_string()));
            }
        }
        if !self
            .collection
            .discovered_plants
            .iter()
            .all(|id| content.plant(id).is_some())
            || !self
                .collection
                .discovered_animals
                .iter()
                .all(|id| content.animal(id).is_some())
        {
            return Err(StateError::UnknownCollectionSpecies);
        }
        Ok(())
    }

    #[must_use]
    pub fn content_version(&self) -> &str {
        &self.content_version
    }

    #[must_use]
    pub fn wallet(&self) -> &Wallet {
        &self.wallet
    }

    #[must_use]
    pub fn plants(&self) -> &BTreeMap<PlantId, PlantState> {
        &self.plants
    }

    #[must_use]
    pub fn animals(&self) -> &BTreeMap<AnimalId, AnimalState> {
        &self.animals
    }

    #[must_use]
    pub fn collection(&self) -> &CollectionState {
        &self.collection
    }

    #[must_use]
    pub fn total_animal_count(&self) -> BigUint {
        self.animals
            .values()
            .fold(BigUint::zero(), |total, animal| {
                total + animal.count.as_biguint()
            })
    }

    pub(crate) fn wallet_mut(&mut self) -> &mut Wallet {
        &mut self.wallet
    }

    pub(crate) fn plants_mut(&mut self) -> &mut BTreeMap<PlantId, PlantState> {
        &mut self.plants
    }

    pub(crate) fn animals_mut(&mut self) -> &mut BTreeMap<AnimalId, AnimalState> {
        &mut self.animals
    }

    pub(crate) fn collection_mut(&mut self) -> &mut CollectionState {
        &mut self.collection
    }
}

impl CollectionState {
    pub(crate) fn discover_plant(&mut self, id: PlantId) -> bool {
        self.discovered_plants.insert(id)
    }

    pub(crate) fn discover_animal(&mut self, id: AnimalId) -> bool {
        self.discovered_animals.insert(id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transition<T> {
    pub state: GameState,
    pub outcome: T,
    pub events: Vec<DomainEvent>,
}

#[cfg(test)]
mod tests {
    use num_bigint::BigUint;

    use crate::{
        AnimalConfig, AnimalId, Coins, ContentConfig, EmergencyPurchaseRule, GameState,
        PlantConfig, PlantId, PlantState, ProductionRemainder60, StateError, StockCent,
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
    fn zero_count_plant_cannot_restore_nonzero_stock() {
        let (content, plant_id) = fixture();
        let state = GameState::new(&content, Coins::zero(), [], []).unwrap();
        let mut plants = state.plants().clone();
        plants.insert(
            plant_id,
            PlantState {
                count: 0_u64.into(),
                stock_cent: StockCent::from(1_u64),
                production_remainder_60: ProductionRemainder60::default(),
            },
        );
        let restored = GameState::try_from_parts(
            state.content_version().to_owned(),
            state.wallet().clone(),
            plants,
            state.animals().clone(),
            state.collection().clone(),
            &content,
        );
        assert!(matches!(restored, Err(StateError::EmptyPlantHasStock(_))));
    }
}
