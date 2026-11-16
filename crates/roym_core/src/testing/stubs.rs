//! Stub implementations of host traits for `TestHost`.

use syneroym_app_host::{
    AppAppConfig, AppBlobReader, AppBlobStore, AppBlobWriter, AppConversation, AppEvents,
    AppMessaging, AppTranslation, AppVault, AppWebSocket,
    types::{
        app_config::ConfigError,
        blob_store::BlobError,
        conversation::{
            ChangePage, ConversationError, ConversationSummary, DeliveryState, ExportChunk,
            GroupInfo, HistoryPage, Message, Transcript,
        },
        events::{EventsError, ReadResult},
        http::FrameKind,
        messaging::MessagingError,
        translation::{Detected, Translated, TranslationError},
        vault::VaultError,
    },
};

use crate::testing::TestHost;

#[derive(Debug)]
pub struct DummyWriter;

impl AppBlobWriter for DummyWriter {
    async fn write(&mut self, _chunk: Vec<u8>) -> Result<(), BlobError> {
        unimplemented!()
    }
    async fn finish(self) -> Result<String, BlobError> {
        unimplemented!()
    }
    async fn abort(self) {}
}

#[derive(Debug)]
pub struct DummyReader;

impl AppBlobReader for DummyReader {
    async fn read(&mut self, _max_bytes: u32) -> Result<Vec<u8>, BlobError> {
        unimplemented!()
    }
}

impl AppBlobStore for TestHost {
    type Writer = DummyWriter;
    type Reader = DummyReader;

    async fn put_blob(&self, _data: Vec<u8>) -> Result<String, BlobError> {
        unimplemented!()
    }
    async fn get_blob(&self, _hash: String) -> Result<Vec<u8>, BlobError> {
        unimplemented!()
    }
    async fn open_upload(&self) -> Result<DummyWriter, BlobError> {
        unimplemented!()
    }
    async fn open_download(&self, _hash: String, _offset: u64) -> Result<DummyReader, BlobError> {
        unimplemented!()
    }
    async fn delete_blob(&self, _hash: String) -> Result<(), BlobError> {
        unimplemented!()
    }
    async fn signed_url(&self, _hash: String, _ttl: u32) -> Result<String, BlobError> {
        unimplemented!()
    }
}

impl AppMessaging for TestHost {
    async fn publish(&self, _topic: String, _payload: Vec<u8>) -> Result<(), MessagingError> {
        unimplemented!()
    }
    async fn subscribe(&self, _topic: String) -> Result<(), MessagingError> {
        unimplemented!()
    }
    async fn unsubscribe(&self, _topic: String) -> Result<(), MessagingError> {
        unimplemented!()
    }
}

impl AppAppConfig for TestHost {
    async fn get(&self, _key: String) -> Result<Option<String>, ConfigError> {
        unimplemented!()
    }
    async fn get_section(&self, _prefix: String) -> Result<Vec<(String, String)>, ConfigError> {
        unimplemented!()
    }
}

impl AppVault for TestHost {
    async fn reveal(&self, _key: String) -> Result<Vec<u8>, VaultError> {
        unimplemented!()
    }
}

impl AppWebSocket for TestHost {
    async fn send(&self, _conn: String, _frame: Vec<u8>, _kind: FrameKind) -> Result<(), String> {
        unimplemented!()
    }
}

impl AppEvents for TestHost {
    async fn record_event(
        &self,
        _kind: String,
        _subject: String,
        _item: Option<String>,
    ) -> Result<String, EventsError> {
        Ok("0000000000000000-1".to_string())
    }

    async fn read_events(
        &self,
        _after: Option<String>,
        _limit: u32,
    ) -> Result<ReadResult, EventsError> {
        Ok(ReadResult::Events((Vec::new(), "0000000000000000-1".to_string())))
    }
}

impl AppTranslation for TestHost {
    async fn supported_targets(&self) -> Vec<String> {
        Vec::new()
    }

    async fn detect_language(&self, _text: String) -> Result<Vec<Detected>, TranslationError> {
        Err(TranslationError::Unavailable)
    }

    async fn translate(
        &self,
        _text: String,
        _source_lang: Option<String>,
        _target_lang: String,
    ) -> Result<Translated, TranslationError> {
        Err(TranslationError::Unavailable)
    }
}

impl AppConversation for TestHost {
    async fn open_direct(&self, _peer: String) -> Result<String, ConversationError> {
        unimplemented!()
    }
    async fn conversations(&self) -> Result<Vec<ConversationSummary>, ConversationError> {
        unimplemented!()
    }
    async fn send(
        &self,
        _cid: String,
        _ctype: String,
        _languages: Vec<String>,
        _body: Vec<u8>,
    ) -> Result<String, ConversationError> {
        unimplemented!()
    }
    async fn history(
        &self,
        _cid: String,
        _limit: u32,
        _cursor: Option<String>,
    ) -> Result<HistoryPage, ConversationError> {
        unimplemented!()
    }
    async fn delivery_status(&self, _mid: String) -> Result<DeliveryState, ConversationError> {
        unimplemented!()
    }
    async fn outbox(&self) -> Result<Vec<Message>, ConversationError> {
        unimplemented!()
    }
    async fn retry(&self, _mid: String) -> Result<(), ConversationError> {
        unimplemented!()
    }
    async fn create_group(&self) -> Result<String, ConversationError> {
        unimplemented!()
    }
    async fn add_member(&self, _cid: String, _member: String) -> Result<(), ConversationError> {
        unimplemented!()
    }
    async fn remove_member(&self, _cid: String, _member: String) -> Result<(), ConversationError> {
        unimplemented!()
    }
    async fn members(&self, _cid: String) -> Result<Vec<String>, ConversationError> {
        unimplemented!()
    }
    async fn sync_now(&self, _cid: String) -> Result<(), ConversationError> {
        unimplemented!()
    }
    async fn group_info(&self, _cid: String) -> Result<GroupInfo, ConversationError> {
        Err(ConversationError::NotFound)
    }
    async fn get_message(&self, _mid: String) -> Result<Message, ConversationError> {
        Err(ConversationError::NotFound)
    }
    async fn delete_message(
        &self,
        _message: String,
        _ask_others: bool,
    ) -> Result<(), ConversationError> {
        unimplemented!()
    }
    async fn readmit(
        &self,
        _conversation: String,
        _reasons: Vec<String>,
    ) -> Result<u32, ConversationError> {
        unimplemented!()
    }
    async fn changes(
        &self,
        _conversation: String,
        _after_seq: u64,
        _limit: u32,
    ) -> Result<ChangePage, ConversationError> {
        unimplemented!()
    }
    async fn search(
        &self,
        _query: String,
        _conversation: Option<String>,
        _limit: u32,
    ) -> Result<Vec<Message>, ConversationError> {
        unimplemented!()
    }
    async fn set_group_name(
        &self,
        _conversation: String,
        _name: String,
    ) -> Result<(), ConversationError> {
        unimplemented!()
    }
    async fn transcript_digest(
        &self,
        _conversation: String,
    ) -> Result<Transcript, ConversationError> {
        unimplemented!()
    }
    async fn export_history(
        &self,
        _cursor: Option<String>,
    ) -> Result<ExportChunk, ConversationError> {
        unimplemented!()
    }
    async fn import_history(&self, _data: Vec<u8>) -> Result<u32, ConversationError> {
        unimplemented!()
    }
}
