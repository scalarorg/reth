use revm_primitives::B256;

use super::storage::InMemoryStorage;

pub trait ParallelEvmContextTrait {
    fn storage(&self) -> &InMemoryStorage<'_>;
    fn set_storage(&mut self, storage: InMemoryStorage<'static>);
    fn set_block_hash(&mut self, number: u64, hash: B256);
}
#[derive(Default, Debug, Clone)]
pub struct ParallelEvmContext {
    storage: InMemoryStorage<'static>,
}
impl ParallelEvmContext {
    pub fn new(storage: InMemoryStorage<'static>) -> Self {
        ParallelEvmContext { storage }
    }
}

impl ParallelEvmContextTrait for ParallelEvmContext {
    fn storage(&self) -> &InMemoryStorage<'_> {
        &self.storage
    }
    fn set_storage(&mut self, storage: InMemoryStorage<'static>) {
        self.storage = storage;
    }
    fn set_block_hash(&mut self, number: u64, hash: B256) {
        self.storage.set_block_hash(number, hash);
    }
}
