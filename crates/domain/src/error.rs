use thiserror::Error;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("content version must not be empty")]
    EmptyContentVersion,
    #[error("species id must not be empty")]
    EmptySpeciesId,
    #[error("species id contains unsupported characters: {0}")]
    InvalidSpeciesId(String),
    #[error("content must contain at least one plant and one animal")]
    EmptyCatalog,
    #[error("duplicate plant id: {0}")]
    DuplicatePlantId(String),
    #[error("duplicate animal id: {0}")]
    DuplicateAnimalId(String),
    #[error("duplicate land slot {0}")]
    DuplicateLandSlot(u8),
    #[error("land slot {0} is outside the five MVP land slots")]
    InvalidLandSlot(u8),
    #[error("plant {0} has an invalid zero-valued parameter")]
    InvalidPlantParameter(String),
    #[error("animal {0} has an invalid parameter")]
    InvalidAnimalParameter(String),
    #[error("plant {plant_id} references unknown animal {animal_id}")]
    UnknownPairedAnimal { plant_id: String, animal_id: String },
    #[error("animal {animal_id} references unknown plant {plant_id}")]
    UnknownFoodPlant { animal_id: String, plant_id: String },
    #[error("plant/animal pairing is not reciprocal for {plant_id} and {animal_id}")]
    NonReciprocalPair { plant_id: String, animal_id: String },
    #[error("emergency animal is not present: {0}")]
    UnknownEmergencyAnimal(String),
    #[error("emergency threshold must equal the animal's normal fixed purchase price")]
    InvalidEmergencyThreshold,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum StateError {
    #[error("state content version {state} does not match catalog {catalog}")]
    ContentVersionMismatch { state: String, catalog: String },
    #[error("state does not contain exactly the configured species")]
    SpeciesSetMismatch,
    #[error("unknown plant: {0}")]
    UnknownPlant(String),
    #[error("unknown animal: {0}")]
    UnknownAnimal(String),
    #[error("duplicate initial grant: {0}")]
    DuplicateInitialGrant(String),
    #[error("plant production remainder must be in 0..=59")]
    InvalidProductionRemainder,
    #[error("a plant with zero count cannot retain production remainder: {0}")]
    EmptyPlantHasProductionRemainder(String),
    #[error("a plant with zero count cannot retain stock: {0}")]
    EmptyPlantHasStock(String),
    #[error("feeding progress for {animal_id} must be below threshold {threshold}")]
    InvalidFeedingProgress { animal_id: String, threshold: u8 },
    #[error("an empty animal group cannot retain growth or feeding progress: {0}")]
    EmptyAnimalHasProgress(String),
    #[error("collection contains an unknown species")]
    UnknownCollectionSpecies,
    #[error("an owned or historically purchased species must be discovered: {0}")]
    OwnedSpeciesUndiscovered(String),
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum DomainError {
    #[error(transparent)]
    InvalidState(#[from] StateError),
    #[error("selection is empty")]
    EmptySelection,
    #[error("duplicate selection: {0}")]
    DuplicateSelection(String),
    #[error("unknown plant: {0}")]
    UnknownPlant(String),
    #[error("unknown animal: {0}")]
    UnknownAnimal(String),
    #[error("insufficient coins")]
    InsufficientCoins,
    #[error("sale quantity exceeds the owned animal count: {0}")]
    SaleQuantityExceedsOwned(String),
    #[error("emergency rabbit quantity must be exactly one")]
    InvalidEmergencyQuantity,
    #[error("purchase contains {requested} plant units, above the command pricing budget {limit}")]
    PlantPricingTotalUnitsBudgetExceeded { requested: String, limit: u64 },
    #[error("plant {plant_id} would reach sequence {requested}, above the pricing budget {limit}")]
    PlantPricingSequenceBudgetExceeded {
        plant_id: String,
        requested: String,
        limit: u64,
    },
    #[error(
        "plant {plant_id} pricing needs about {estimated} intermediate decimal digits, above the budget {limit}"
    )]
    PlantPricingDigitsBudgetExceeded {
        plant_id: String,
        estimated: u64,
        limit: u64,
    },
    #[error("purchase needs about {estimated} digit-work units, above the command budget {limit}")]
    PlantPricingWorkBudgetExceeded { estimated: u64, limit: u64 },
    #[error("plant sequence numbers are one-based")]
    InvalidPlantSequenceNumber,
}
