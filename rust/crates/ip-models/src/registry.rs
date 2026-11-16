//! 按名字找模型。岗位配置里的模型名写成 `<端点>/<模型>`，例如 `deepseek/deepseek-chat`、
//! `local/qwen-1b+lora-finance-ap`；斜杠前是端点名，后面原样交给端点。

use std::collections::HashMap;
use std::sync::Arc;

use crate::{ModelClient, Price};

#[derive(Default, Clone)]
pub struct ModelRegistry {
    endpoints: HashMap<String, Arc<dyn ModelClient>>,
    /// 按完整模型名（`<端点>/<模型>`）登记的价格。
    prices: HashMap<String, Price>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ResolveError {
    #[error("模型名 {0} 缺少端点前缀，应写成 <端点>/<模型>")]
    MissingEndpoint(String),
    #[error("没有配置名为 {0} 的模型端点")]
    UnknownEndpoint(String),
}

impl ModelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, name: impl Into<String>, client: Arc<dyn ModelClient>) {
        self.endpoints.insert(name.into(), client);
    }

    /// 登记一个模型的价格；`model` 是完整名 `<端点>/<模型>`。
    pub fn set_price(&mut self, model: impl Into<String>, price: Price) {
        self.prices.insert(model.into(), price);
    }

    pub fn price(&self, model: &str) -> Option<&Price> {
        self.prices.get(model)
    }

    /// 返回端点和端点内的模型名。
    pub fn resolve<'a>(&self, model: &'a str) -> Result<(Arc<dyn ModelClient>, &'a str), ResolveError> {
        let (endpoint, name) = model
            .split_once('/')
            .ok_or_else(|| ResolveError::MissingEndpoint(model.to_owned()))?;
        let client = self
            .endpoints
            .get(endpoint)
            .ok_or_else(|| ResolveError::UnknownEndpoint(endpoint.to_owned()))?;
        Ok((Arc::clone(client), name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ScriptedModel;

    #[test]
    fn resolves_endpoint_prefixes() {
        let mut registry = ModelRegistry::new();
        registry.insert("local", Arc::new(ScriptedModel::new([])));
        let (_, name) = registry.resolve("local/qwen-1b+lora-finance-ap").unwrap();
        assert_eq!(name, "qwen-1b+lora-finance-ap");
        assert_eq!(
            registry.resolve("gpt").err(),
            Some(ResolveError::MissingEndpoint("gpt".into()))
        );
        assert_eq!(
            registry.resolve("cloud/x").err(),
            Some(ResolveError::UnknownEndpoint("cloud".into()))
        );
    }
}
