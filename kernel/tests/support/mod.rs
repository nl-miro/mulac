#![allow(dead_code)]
use mulac_kernel::io::*;
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
pub struct Start;
impl ApplicationCommand for Start {
    fn command_type(&self) -> &'static str {
        "start"
    }
}
#[derive(Serialize, Deserialize)]
pub struct Follow;
impl ApplicationCommand for Follow {
    fn command_type(&self) -> &'static str {
        "follow"
    }
}
#[derive(Serialize)]
pub struct Started;
impl ApplicationEvent for Started {
    fn event_type(&self) -> &'static str {
        "started"
    }
    fn routing_key(&self) -> Option<&str> {
        Some("qa01.started")
    }
}
pub struct StartHandler;
impl CommandHandlerPort<Start, Started> for StartHandler {
    fn execute(&self, _: Start) -> Result<Vec<Started>, CommandError> {
        Ok(vec![Started])
    }
}
pub struct FollowHandler(pub Arc<AtomicUsize>);
impl CommandHandlerPort<Follow, Started> for FollowHandler {
    fn execute(&self, _: Follow) -> Result<Vec<Started>, CommandError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(vec![])
    }
}
pub struct Capture(pub Arc<Mutex<Vec<NewEventEnvelope>>>);
impl EventSubscriberPort for Capture {
    fn handle(&self, event: &NewEventEnvelope) -> Result<(), EventError> {
        self.0.lock().unwrap().push(event.clone());
        Ok(())
    }
}
pub struct FollowSubscriber(pub Arc<CommandGateway>);
impl EventSubscriberPort for FollowSubscriber {
    fn handle(&self, event: &NewEventEnvelope) -> Result<(), EventError> {
        let event_meta = event.metadata.as_ref().unwrap();
        let envelope = NewCommandEnvelope {
            command: Follow,
            metadata: NewCommandMetadata {
                command_id: Uuid::now_v7(),
                correlation_id: event_meta.correlation_id,
                causation_id: Some(event_meta.event_id),
                source: Some("subscriber".into()),
            },
        }
        .into_gateway_envelope()
        .unwrap();
        self.0
            .dispatch(envelope)
            .map_err(|error| EventError::SubscriberExecution(error.to_string()))
    }
}
pub fn envelope<C>(command: C) -> NewCommandEnvelope<C> {
    NewCommandEnvelope {
        command,
        metadata: NewCommandMetadata {
            command_id: Uuid::now_v7(),
            correlation_id: None,
            causation_id: None,
            source: None,
        },
    }
}
pub fn event(event_type: &str) -> NewEventEnvelope {
    NewEventEnvelope {
        event_type: event_type.into(),
        payload: "{\"value\":42}".into(),
        metadata: Some(NewEventMetadata {
            event_id: Uuid::now_v7(),
            correlation_id: Some(Uuid::now_v7()),
            causation_id: Some(Uuid::now_v7()),
            source: Some("qa01".into()),
            routing_key: Some("qa01.started".into()),
        }),
    }
}
