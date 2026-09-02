//! Deterministic, I/O-free domain rules for Click Clack Farm.
//!
//! All authoritative economic values are non-negative integers. Plant stock and
//! animal growth use cent-units (`0.01`) and never pass through floating point.

mod amount;
mod config;
mod error;
mod event;
mod input;
mod production;
mod purchase;
mod sale;
mod state;

pub use amount::{Coins, EntityCount, GrowthCent, LifetimePurchaseCount, StockCent};
pub use config::{
    AnimalConfig, AnimalId, ContentConfig, EmergencyPurchaseRule, PlantConfig, PlantId,
};
pub use error::{ConfigError, DomainError, StateError};
pub use event::DomainEvent;
pub use input::{FeedingSettlement, InputSettlement, apply_effective_inputs};
pub use production::{
    PlantProduction, ProductionSettlement, settle_production, single_plant_price,
    single_plant_price_with_budget,
};
pub use purchase::{
    DEFAULT_MAX_ESTIMATED_DIGIT_WORK, DEFAULT_MAX_ESTIMATED_PRICE_DIGITS,
    DEFAULT_MAX_PLANT_SEQUENCE_NUMBER, DEFAULT_MAX_PLANT_UNITS_PER_BATCH, PurchaseKind,
    PurchaseLine, PurchasePricingBudget, PurchaseQuote, PurchaseReceipt, PurchaseSelection,
    apply_purchase_batch, apply_purchase_batch_with_budget, quote_purchase_batch,
    quote_purchase_batch_with_budget,
};
pub use sale::{
    SaleLine, SaleQuote, SaleReceipt, SaleSelection, apply_sale_batch, quote_sale_batch,
};
pub use state::{
    AnimalState, CollectionState, FeedingProgress, GameState, PlantState, ProductionRemainder60,
    Transition, Wallet,
};
