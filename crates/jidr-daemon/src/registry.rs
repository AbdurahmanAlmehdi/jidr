//! The command table built from every provider's `CommandSpec`s.

use std::collections::BTreeMap;
use std::sync::Arc;

use jidr_core::provider::{CommandProvider, CommandSpec};
use jidr_core::{JidrError, JidrResult};

#[derive(Default)]
pub struct Registry {
    providers: Vec<Arc<dyn CommandProvider>>,
    table: BTreeMap<String, (usize, CommandSpec)>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("commands", &self.table.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// Methods the server handles itself (not provider commands).
pub const BUILTIN_METHODS: &[&str] = &[
    "hello",
    "commands.list",
    "events.subscribe",
    "logcat.follow",
    "daemon.shutdown",
];

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a provider; a command id that already exists is an error.
    pub fn register(&mut self, provider: Arc<dyn CommandProvider>) -> JidrResult<()> {
        let specs = provider.commands();
        for s in &specs {
            if self.table.contains_key(&s.id) || BUILTIN_METHODS.contains(&s.id.as_str()) {
                return Err(JidrError::unexpected(format!(
                    "provider {} registers {}, which already exists",
                    provider.name(),
                    s.id
                )));
            }
        }
        let index = self.providers.len();
        self.providers.push(provider);
        for s in specs {
            self.table.insert(s.id.clone(), (index, s));
        }
        Ok(())
    }

    pub fn get(&self, method: &str) -> Option<(&Arc<dyn CommandProvider>, &CommandSpec)> {
        let (i, spec) = self.table.get(method)?;
        Some((self.providers.get(*i)?, spec))
    }

    /// Every command, sorted by id.
    pub fn list(&self) -> Vec<CommandSpec> {
        self.table.values().map(|(_, s)| s.clone()).collect()
    }

    pub fn providers(&self) -> &[Arc<dyn CommandProvider>] {
        &self.providers
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use jidr_core::provider::{CallCtx, NoParams};
    use serde_json::Value;

    struct P(&'static str, &'static [&'static str]);

    #[async_trait]
    impl CommandProvider for P {
        fn name(&self) -> &str {
            self.0
        }
        fn commands(&self) -> Vec<CommandSpec> {
            self.1
                .iter()
                .map(|id| CommandSpec::new::<NoParams>(id, "x"))
                .collect()
        }
        async fn call(&self, _: &CallCtx, _: &str, _: Value) -> JidrResult<Value> {
            Ok(Value::Null)
        }
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let mut r = Registry::new();
        r.register(Arc::new(P("a", &["x.one", "x.two"]))).unwrap();
        assert!(r.register(Arc::new(P("b", &["x.two"]))).is_err());
        assert!(r.register(Arc::new(P("c", &["hello"]))).is_err());
        r.register(Arc::new(P("d", &["y.one"]))).unwrap();
        let ids: Vec<String> = r.list().into_iter().map(|s| s.id).collect();
        assert_eq!(ids, ["x.one", "x.two", "y.one"]);
        assert_eq!(r.get("y.one").unwrap().0.name(), "d");
        assert!(r.get("nope").is_none());
    }
}
