use mulac_kernel::{ApplicationCommand, ApplicationEvent};

#[derive(serde::Serialize, ApplicationCommand)]
struct CreateThing {
    name: String,
}

#[derive(serde::Serialize, ApplicationEvent)]
struct ThingCreated {
    name: String,
}

#[test]
fn derives_type_names_from_struct_name() {
    let command = CreateThing { name: "a".into() };
    let event = ThingCreated { name: "a".into() };

    assert_eq!(command.command_type(), "CreateThing");
    assert_eq!(event.event_type(), "ThingCreated");
}
