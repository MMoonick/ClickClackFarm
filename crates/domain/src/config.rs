use std::{collections::BTreeMap, fmt};

use num_bigint::BigUint;
use num_traits::Zero;

use crate::{Coins, ConfigError, StockCent};

macro_rules! species_id {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, ConfigError> {
                let value = value.into();
                if value.is_empty() {
                    return Err(ConfigError::EmptySpeciesId);
                }
                if !value
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
                {
                    return Err(ConfigError::InvalidSpeciesId(value));
                }
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

species_id!(PlantId);
species_id!(AnimalId);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlantConfig {
    pub id: PlantId,
    pub fixed_land_slot: u8,
    pub base_price_coins: Coins,
    pub price_growth_num: BigUint,
    pub price_growth_den: BigUint,
    pub rate_cent_per_minute: StockCent,
    pub paired_animal_id: AnimalId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnimalConfig {
    pub id: AnimalId,
    pub fixed_purchase_price_coins: Coins,
    pub zero_growth_sell_price_coins: Coins,
    pub feeding_threshold: u8,
    pub bite_cent: StockCent,
    pub food_plant_id: PlantId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmergencyPurchaseRule {
    pub animal_id: AnimalId,
    pub trigger_below_coins: Coins,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentConfig {
    content_version: String,
    plants: BTreeMap<PlantId, PlantConfig>,
    animals: BTreeMap<AnimalId, AnimalConfig>,
    emergency_purchase: EmergencyPurchaseRule,
}

impl ContentConfig {
    pub fn try_new(
        content_version: impl Into<String>,
        plants: impl IntoIterator<Item = PlantConfig>,
        animals: impl IntoIterator<Item = AnimalConfig>,
        emergency_purchase: EmergencyPurchaseRule,
    ) -> Result<Self, ConfigError> {
        let content_version = content_version.into();
        if content_version.trim().is_empty() {
            return Err(ConfigError::EmptyContentVersion);
        }

        let mut plant_map = BTreeMap::new();
        for plant in plants {
            let id = plant.id.clone();
            if plant_map.insert(id.clone(), plant).is_some() {
                return Err(ConfigError::DuplicatePlantId(id.to_string()));
            }
        }
        let mut animal_map = BTreeMap::new();
        for animal in animals {
            let id = animal.id.clone();
            if animal_map.insert(id.clone(), animal).is_some() {
                return Err(ConfigError::DuplicateAnimalId(id.to_string()));
            }
        }
        let plants = plant_map;
        let animals = animal_map;
        if plants.is_empty() || animals.is_empty() {
            return Err(ConfigError::EmptyCatalog);
        }

        let config = Self {
            content_version,
            plants,
            animals,
            emergency_purchase,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        let mut occupied_slots = [false; 5];
        for plant in self.plants.values() {
            if plant.fixed_land_slot >= 5 {
                return Err(ConfigError::InvalidLandSlot(plant.fixed_land_slot));
            }
            let slot = usize::from(plant.fixed_land_slot);
            if occupied_slots[slot] {
                return Err(ConfigError::DuplicateLandSlot(plant.fixed_land_slot));
            }
            occupied_slots[slot] = true;
            if plant.base_price_coins.is_zero()
                || plant.price_growth_num.is_zero()
                || plant.price_growth_den.is_zero()
                || plant.price_growth_num < plant.price_growth_den
                || plant.rate_cent_per_minute.is_zero()
            {
                return Err(ConfigError::InvalidPlantParameter(plant.id.to_string()));
            }
            let animal = self.animals.get(&plant.paired_animal_id).ok_or_else(|| {
                ConfigError::UnknownPairedAnimal {
                    plant_id: plant.id.to_string(),
                    animal_id: plant.paired_animal_id.to_string(),
                }
            })?;
            if animal.food_plant_id != plant.id {
                return Err(ConfigError::NonReciprocalPair {
                    plant_id: plant.id.to_string(),
                    animal_id: animal.id.to_string(),
                });
            }
        }

        for animal in self.animals.values() {
            if animal.fixed_purchase_price_coins.is_zero()
                || animal.feeding_threshold == 0
                || animal.bite_cent.is_zero()
                || animal.zero_growth_sell_price_coins.as_biguint()
                    >= animal.fixed_purchase_price_coins.as_biguint()
            {
                return Err(ConfigError::InvalidAnimalParameter(animal.id.to_string()));
            }
            let plant = self.plants.get(&animal.food_plant_id).ok_or_else(|| {
                ConfigError::UnknownFoodPlant {
                    animal_id: animal.id.to_string(),
                    plant_id: animal.food_plant_id.to_string(),
                }
            })?;
            if plant.paired_animal_id != animal.id {
                return Err(ConfigError::NonReciprocalPair {
                    plant_id: plant.id.to_string(),
                    animal_id: animal.id.to_string(),
                });
            }
        }

        let emergency_animal = self
            .animals
            .get(&self.emergency_purchase.animal_id)
            .ok_or_else(|| {
                ConfigError::UnknownEmergencyAnimal(self.emergency_purchase.animal_id.to_string())
            })?;
        if self.emergency_purchase.trigger_below_coins
            != emergency_animal.fixed_purchase_price_coins
            || emergency_animal.zero_growth_sell_price_coins.is_zero()
        {
            return Err(ConfigError::InvalidEmergencyThreshold);
        }
        Ok(())
    }

    #[must_use]
    pub fn content_version(&self) -> &str {
        &self.content_version
    }

    #[must_use]
    pub fn plants(&self) -> &BTreeMap<PlantId, PlantConfig> {
        &self.plants
    }

    #[must_use]
    pub fn animals(&self) -> &BTreeMap<AnimalId, AnimalConfig> {
        &self.animals
    }

    #[must_use]
    pub fn plant(&self, id: &PlantId) -> Option<&PlantConfig> {
        self.plants.get(id)
    }

    #[must_use]
    pub fn animal(&self, id: &AnimalId) -> Option<&AnimalConfig> {
        self.animals.get(id)
    }

    #[must_use]
    pub fn emergency_purchase(&self) -> &EmergencyPurchaseRule {
        &self.emergency_purchase
    }
}
