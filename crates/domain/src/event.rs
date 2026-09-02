use crate::{AnimalId, InputSettlement, PlantId, ProductionSettlement, PurchaseQuote, SaleQuote};

/// The sole authoritative source of committed domain events for WP3.
///
/// Command outcomes are retained as convenient local receipts. Downstream
/// layers must persist or publish these events from `Transition::events`
/// instead of reconstructing them by diffing state or interpreting outcomes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DomainEvent {
    ProductionSettled(ProductionSettlement),
    PurchaseBatchCommitted(PurchaseQuote),
    FeedingAttempted(InputSettlement),
    AnimalsSoldBatch(SaleQuote),
    CollectionDiscovered {
        plants: Vec<PlantId>,
        animals: Vec<AnimalId>,
    },
}
