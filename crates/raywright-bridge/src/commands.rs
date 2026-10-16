use std::collections::BTreeMap;

use raywright_protocol::{CallParams, RpcError, codes};
use serde_json::Value;

type Command<C> = Box<dyn Fn(&mut C, Value) -> Result<Value, String> + Send + Sync>;

/// The app's own commands for `call`, run against the app's state `C` on whatever thread
/// owns it (usually the UI thread). One line per command:
///
/// ```
/// # use raywright_bridge::CommandRegistry;
/// struct App { edit: bool }
/// let mut cmds = CommandRegistry::<App>::new();
/// cmds.add("set_edit", "turn edit mode on or off", |app, args| {
///     app.edit = args["on"].as_bool().unwrap_or(true);
///     Ok(serde_json::json!({ "edit": app.edit }))
/// });
/// let mut app = App { edit: false };
/// cmds.dispatch(&mut app, "set_edit", serde_json::json!({ "on": true })).unwrap();
/// assert!(app.edit);
/// ```
pub struct CommandRegistry<C> {
    commands: BTreeMap<String, (String, Command<C>)>,
}

impl<C> Default for CommandRegistry<C> {
    fn default() -> Self {
        CommandRegistry { commands: BTreeMap::new() }
    }
}

impl<C> CommandRegistry<C> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, name: &str, help: &str, f: impl Fn(&mut C, Value) -> Result<Value, String> + Send + Sync + 'static) -> &mut Self {
        self.commands.insert(name.to_string(), (help.to_string(), Box::new(f)));
        self
    }

    /// For [`crate::BridgeBuilder::commands`].
    pub fn names(&self) -> Vec<String> {
        self.commands.keys().cloned().collect()
    }

    /// `(name, help)` of every command.
    pub fn help(&self) -> Vec<(String, String)> {
        self.commands.iter().map(|(n, (h, _))| (n.clone(), h.clone())).collect()
    }

    pub fn dispatch(&self, ctx: &mut C, name: &str, args: Value) -> Result<Value, RpcError> {
        match self.commands.get(name) {
            Some((_, f)) => f(ctx, args).map_err(RpcError::app),
            None => Err(RpcError::new(codes::METHOD_NOT_FOUND, format!("no command `{name}` (have: {})", self.names().join(", ")))),
        }
    }

    /// Runs the command a `call` request names.
    pub fn dispatch_call(&self, ctx: &mut C, params: &Value) -> Result<Value, RpcError> {
        let p: CallParams = raywright_protocol::params(params)?;
        self.dispatch(ctx, &p.name, p.args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dispatches_and_reports_unknown_commands() {
        let mut r = CommandRegistry::<u32>::new();
        r.add("inc", "add one", |n, _| {
            *n += 1;
            Ok(json!(*n))
        });
        let mut n = 0;
        assert_eq!(r.dispatch_call(&mut n, &json!({"name": "inc"})).unwrap(), json!(1));
        let e = r.dispatch(&mut n, "dec", Value::Null).unwrap_err();
        assert!(e.message.contains("have: inc"));
        assert_eq!(r.help(), vec![("inc".to_string(), "add one".to_string())]);
    }
}
