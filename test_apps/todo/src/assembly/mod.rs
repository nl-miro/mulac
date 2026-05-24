mod application;
mod domain;
mod infra_sqlx_pg;

pub mod io {
    pub use super::application::{
        ApiError,
        AppCommand,
        AppError,
        Command,
        ErrorBody,
        MulacHandle,
        MulacState,
        NewCommandEnvelope,
        block_on_blocking,
        interpret_dispatch_error,
        run_command_worker,
        run_event_worker,
        start_mulac,
        validate_title,
        //
    };
    pub use super::domain::{Clock, TodoDto, TodoList, TodoStatus};
    pub use super::infra_sqlx_pg::entity::TodoRow;
    pub use super::infra_sqlx_pg::{
        OutboxSubscriber,
        connect,
        fetch_todo,
        migrate,
        record_event_payload,
        //
    };
    pub use crate::TodoEvent;
}
