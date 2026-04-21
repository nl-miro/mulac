//! Inbox storage component

struct InboxEntry {}

struct InboxStorageError {}

struct InboxStorage {}

impl InboxStorage {
    pub fn store(&self, entry: InboxEntry) -> Result<(), InboxStorageError> {
        let _ = entry;
        // TODO: implement

        Ok(())
    }

    pub fn reserve(&self, cnt: i32) -> Result<Vec<InboxEntry>, InboxStorageError> {
        let _ = cnt;
        // TODO: implement

        Ok(vec![])
    }
}
