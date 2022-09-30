mod locally_shuffled;
pub mod old;
mod sequential;
pub trait TransactionIds<T> {
    fn generate(&mut self) -> T;
}

pub use self::locally_shuffled::LocallyShuffledIds;
pub use self::sequential::SequentialIds;
