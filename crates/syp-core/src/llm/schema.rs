use serde_json::Value;

#[derive(Debug, Clone)]
pub struct JsonResponseSchema {
    name: &'static str,
    schema: Value,
}

impl JsonResponseSchema {
    #[must_use]
    pub fn new(name: &'static str, schema: Value) -> Self {
        Self { name, schema }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        self.name
    }

    #[must_use]
    pub fn schema(&self) -> &Value {
        &self.schema
    }
}
