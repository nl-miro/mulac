# Component High-Level View

Use this template to document a component at the architecture level. Keep it focused on behaviour and models, not implementation details.

## {{ Component Name }}

**Role:**
Describe what the component is responsible for in the system and where it sits in the flow.

**Behaviour:**
1. Describe how work enters the component.
2. Describe how the component stores, transforms, routes, or delivers that work.
3. Name the concrete actor if multiple actors collaborate, for example `service`, `consumer`, or `dispatcher`.
4. State when handoff to the next component happens.
5. State when work is marked as `completed`, if the component has durable lifecycle tracking.

**Model:**
List the main model or models the component works with and what each one represents.

**States:**
Use this section only if the component has a durable lifecycle.

1. `received` - the item was accepted and stored.
2. `reserved` - a consumer claimed the item for processing.
3. `completed` - the item was successfully handed off or delivered.
4. `failed` - processing or delivery failed and the item is waiting for its next scheduled retry time.
5. `dead` - the retry policy was exhausted and the item will not be retried automatically.

**Rules:**
1. State the durability boundary, for example when acknowledgement or acceptance is allowed.
2. State concurrency guarantees, for example that a reserved item should not be processed by more than one consumer at the same time.
3. State failure handling and retry semantics.
4. State retry limit or dead-letter semantics.
5. State metadata propagation requirements.
6. State at-least-once or idempotency requirements when retries can cause repeated execution or delivery.
7. State what completion means in terms of responsibility moving to the next component.
