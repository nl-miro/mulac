use thiserror::Error;

pub struct CommandGateway {
    // TODO: implement . spec is write_side/SPEC.md
}

impl CommandGateway {
    pub fn publish(&self, cmd: Command) -> Result<(), CommandError> {
        Err(CommandError::NotImplemented)
    }
}
pub enum Command {
    Publish(String),
}

#[derive(Debug, Error)]
pub enum CommandError {
    #[error("Not implemented")]
    NotImplemented,
}
