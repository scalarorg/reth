use revm_primitives::B256;

use super::storage::InMemoryStorage;

pub trait ParallelEvmContextTrait {
    fn storage(&self) -> &InMemoryStorage<'_>;
    fn set_block_hash(&mut self, number: u64, hash: B256);
}
#[derive(Default, Clone)]
pub struct ParallelEvmContext<'a> {
    storage: InMemoryStorage<'a>,
}
impl<'a> ParallelEvmContext<'a> {
    pub fn new(storage: InMemoryStorage<'a>) -> Self {
        ParallelEvmContext { storage }
    }
}

impl ParallelEvmContextTrait for ParallelEvmContext<'_> {
    fn storage(&self) -> &InMemoryStorage<'_> {
        &self.storage
    }
    fn set_block_hash(&mut self, number: u64, hash: B256) {
        self.storage.set_block_hash(number, hash);
    }
}
