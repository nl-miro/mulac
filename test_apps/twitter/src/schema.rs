// @generated automatically by Diesel CLI.

diesel::table! {
    command_entries (id) {
        id -> Uuid,
        command_type -> Text,
        status -> Int4,
        payload -> Text,
        meta -> Nullable<Jsonb>,
        scheduled_at -> Timestamptz,
        attempts -> Int4,
        reservation_id -> Nullable<Uuid>,
        reserved_at -> Nullable<Timestamptz>,
        received_at -> Timestamptz,
        updated_at -> Timestamptz,
        processed_at -> Nullable<Timestamptz>,
        extra_info -> Nullable<Jsonb>,
    }
}

diesel::table! {
    direct_messages (id) {
        id -> Uuid,
        sender_id -> Uuid,
        recipient_id -> Uuid,
        content -> Text,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    event_entries (id) {
        id -> Uuid,
        event_type -> Text,
        status -> Int4,
        payload -> Text,
        meta -> Nullable<Jsonb>,
        scheduled_at -> Timestamptz,
        attempts -> Int4,
        reservation_id -> Nullable<Uuid>,
        reserved_at -> Nullable<Timestamptz>,
        received_at -> Timestamptz,
        updated_at -> Timestamptz,
        processed_at -> Nullable<Timestamptz>,
        extra_info -> Nullable<Jsonb>,
    }
}

diesel::table! {
    follows (follower_id, following_id) {
        follower_id -> Uuid,
        following_id -> Uuid,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    inbox_entries (id) {
        id -> Uuid,
        status -> Int4,
        payload -> Text,
        meta -> Jsonb,
        scheduled_at -> Timestamptz,
        attempts -> Int4,
        reservation_id -> Nullable<Uuid>,
        reserved_at -> Nullable<Timestamptz>,
        received_at -> Timestamptz,
        updated_at -> Timestamptz,
        processed_at -> Nullable<Timestamptz>,
        extra_info -> Nullable<Jsonb>,
    }
}

diesel::table! {
    inbox_messages (id) {
        id -> Uuid,
        message_type -> Text,
        payload -> Jsonb,
        status -> Text,
        received_at -> Timestamptz,
        processed_at -> Nullable<Timestamptz>,
        error -> Nullable<Text>,
    }
}

diesel::table! {
    likes (user_id, tweet_id) {
        user_id -> Uuid,
        tweet_id -> Uuid,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    outbox_entries (id) {
        id -> Uuid,
        status -> Int4,
        payload -> Text,
        meta -> Jsonb,
        scheduled_at -> Timestamptz,
        attempts -> Int4,
        reservation_id -> Nullable<Uuid>,
        reserved_at -> Nullable<Timestamptz>,
        received_at -> Timestamptz,
        updated_at -> Timestamptz,
        processed_at -> Nullable<Timestamptz>,
        last_error -> Nullable<Text>,
        extra_info -> Nullable<Jsonb>,
    }
}

diesel::table! {
    outbox_messages (id) {
        id -> Uuid,
        event_type -> Text,
        payload -> Jsonb,
        status -> Text,
        created_at -> Timestamptz,
        published_at -> Nullable<Timestamptz>,
        attempts -> Int4,
    }
}

diesel::table! {
    timelines (id) {
        id -> Uuid,
        user_id -> Uuid,
        tweet_id -> Uuid,
        author_id -> Uuid,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    tweets (id) {
        id -> Uuid,
        author_id -> Uuid,
        content -> Text,
        retweeted_from -> Nullable<Uuid>,
        created_at -> Timestamptz,
        deleted_at -> Nullable<Timestamptz>,
    }
}

diesel::joinable!(likes -> tweets (tweet_id));

diesel::allow_tables_to_appear_in_same_query!(
    command_entries,
    direct_messages,
    event_entries,
    follows,
    inbox_entries,
    inbox_messages,
    likes,
    outbox_entries,
    outbox_messages,
    timelines,
    tweets,
);
