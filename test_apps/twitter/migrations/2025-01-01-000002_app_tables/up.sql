CREATE TABLE tweets (
    id UUID PRIMARY KEY,
    author_id UUID NOT NULL,
    content TEXT NOT NULL,
    retweeted_from UUID REFERENCES tweets(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at TIMESTAMPTZ
);

CREATE TABLE follows (
    follower_id UUID NOT NULL,
    following_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (follower_id, following_id)
);

CREATE TABLE likes (
    user_id UUID NOT NULL,
    tweet_id UUID NOT NULL REFERENCES tweets(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, tweet_id)
);

CREATE TABLE direct_messages (
    id UUID PRIMARY KEY,
    sender_id UUID NOT NULL,
    recipient_id UUID NOT NULL,
    content TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE timelines (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL,
    tweet_id UUID NOT NULL,
    author_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_timelines_user_id ON timelines (user_id);
