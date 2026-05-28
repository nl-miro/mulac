mod application;
mod domain;
mod infra_sqlx_pg;

pub mod io {
    pub use super::application::{
        ApiError,
        AppError,
        ErrorBody,
        MulacHandle,
        MulacState,
        block_on_blocking,
        dispatch_command,
        interpret_dispatch_error,
        run_command_worker,
        run_event_worker,
        start_mulac,
        validate_title, //
    };
    pub use super::domain::{Clock, TodoEntry, TodoList, TodoStatus};
    pub use super::infra_sqlx_pg::entity::TodoRow;
    pub use super::infra_sqlx_pg::{
        OutboxSubscriber,
        connect,
        fetch_todo,
        insert_todo,
        migrate,
        record_event_payload, //
    };
    pub use crate::TodoEvent;
}
