use eventing::io::{EventError, EventSubscriberPort};
use std::{collections::HashMap, sync::Arc};

use crate::NewEventEnvelope;

pub struct EventSubscriberRegistry {
    subscribers: HashMap<String, Vec<(String, Arc<dyn EventSubscriberPort>)>>,
}

impl EventSubscriberRegistry {
    pub fn from_subscribers(
        subscribers: Vec<(String, String, Arc<dyn EventSubscriberPort>)>,
    ) -> Self {
        let mut by_event: HashMap<String, Vec<(String, Arc<dyn EventSubscriberPort>)>> =
            HashMap::new();

        for (event_type, subscriber_name, subscriber) in subscribers {
            by_event
                .entry(event_type)
                .or_default()
                .push((subscriber_name, subscriber));
        }

        Self {
            subscribers: by_event,
        }
    }
}

impl EventSubscriberPort for EventSubscriberRegistry {
    fn handle(&self, envelope: &NewEventEnvelope) -> Result<(), EventError> {
        for (_, subscriber) in self
            .subscribers
            .get(&envelope.event_type)
            .into_iter()
            .flatten()
        {
            subscriber.handle(envelope)?;
        }
        Ok(())
    }
}
