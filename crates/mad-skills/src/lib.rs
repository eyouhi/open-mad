use anyhow::Result;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::fmt::Debug;

pub mod actions;
pub mod registry;

pub use actions::desktop::DesktopAction;

/// Represents the context in which a skill is executed.
/// This might include access to the screen, input controller, etc.
pub struct SkillContext {
    // We can add fields here as needed, e.g., reference to InputController
    // For now, we might just instantiate controllers inside skills or pass them here.
    // Given mad-core's design, we might need to pass an Arc<Mutex<InputController>> or similar.
    // But mad-core's InputController seems to be self-contained.
}

#[async_trait]
pub trait Skill: Send + Sync + Debug {
    /// The name of the skill (e.g., "click", "type").
    fn name(&self) -> &'static str;

    /// A description of what the skill does.
    fn description(&self) -> &'static str;

    /// The JSON schema of the arguments this skill accepts.
    fn parameters(&self) -> Value;

    /// Execute the skill with the given arguments.
    async fn execute(&self, args: Value) -> Result<Value>;
}

/// A helper struct to define skills more easily with typed arguments.
pub struct TypedSkill<Args, F> {
    name: &'static str,
    description: &'static str,
    func: F,
    _phantom: std::marker::PhantomData<Args>,
}

impl<Args, F> TypedSkill<Args, F> {
    pub fn new(name: &'static str, description: &'static str, func: F) -> Self {
        Self {
            name,
            description,
            func,
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<Args, F> Debug for TypedSkill<Args, F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypedSkill")
            .field("name", &self.name)
            .field("description", &self.description)
            .finish()
    }
}

#[async_trait]
impl<Args, F, Fut> Skill for TypedSkill<Args, F>
where
    Args: DeserializeOwned + JsonSchema + Send + Sync + 'static,
    F: Fn(Args) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<Value>> + Send + 'static,
{
    fn name(&self) -> &'static str {
        self.name
    }

    fn description(&self) -> &'static str {
        self.description
    }

    fn parameters(&self) -> Value {
        let schema = schemars::schema_for!(Args);
        serde_json::to_value(schema).unwrap_or(Value::Null)
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let args: Args = serde_json::from_value(args)?;
        (self.func)(args).await
    }
}
