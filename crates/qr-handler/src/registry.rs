//! Which handlers this build knows.

use std::{collections::BTreeMap, sync::Arc};

use crate::{Handler, HandlerId};

#[derive(Debug, thiserror::Error)]
#[error("two handlers both call themselves {0}")]
pub struct DuplicateHandler(HandlerId);

#[derive(Clone, Default)]
pub struct Registry {
    handlers: Arc<BTreeMap<HandlerId, Arc<dyn Handler>>>,
}

impl Registry {
    pub fn builder() -> RegistryBuilder {
        RegistryBuilder::default()
    }

    pub fn get(&self, id: &HandlerId) -> Option<&Arc<dyn Handler>> {
        self.handlers.get(id)
    }

    pub fn by_name(&self, id: &str) -> Option<&Arc<dyn Handler>> {
        HandlerId::parse(id).ok().and_then(|id| self.get(&id))
    }

    /// Sorted by id, so the chooser's order does not depend on registration order and
    /// does not change between restarts.
    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn Handler>> {
        self.handlers.values()
    }

    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.handlers.keys()).finish()
    }
}

#[derive(Default)]
pub struct RegistryBuilder {
    handlers: BTreeMap<HandlerId, Arc<dyn Handler>>,
}

impl RegistryBuilder {
    /// Refuses a duplicate rather than replacing one.
    ///
    /// Two handlers answering the same mount path is a build-time mistake, and the one
    /// that lost would be invisible: its routes unreachable, its bindings rendered by
    /// something else.
    pub fn register(mut self, handler: impl Handler) -> Result<Self, DuplicateHandler> {
        let id = handler.id();
        if self.handlers.contains_key(&id) {
            return Err(DuplicateHandler(id));
        }
        self.handlers.insert(id, Arc::new(handler));
        Ok(self)
    }

    pub fn build(self) -> Registry {
        Registry {
            handlers: Arc::new(self.handlers),
        }
    }
}
