use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::Duration,
};

use helix_event::register_hook;
use helix_loader::workspace_trust::TrustQuery;
use helix_vcs::FileBlame;
use helix_view::{
    events::{ConfigDidChange, DocumentDidOpen},
    handlers::{BlameEvent, Handlers},
    DocumentId, Editor,
};
use tokio::time::Instant;

use crate::job;

#[derive(Default)]
pub(super) struct BlameHandler {
    docs: HashSet<DocumentId>,
}

const BLAME_DEBOUNCE: Duration = Duration::from_millis(150);

impl helix_event::AsyncHook for BlameHandler {
    type Event = BlameEvent;

    fn handle_event(&mut self, event: Self::Event, _timeout: Option<Instant>) -> Option<Instant> {
        self.docs.insert(event.0);
        Some(Instant::now() + BLAME_DEBOUNCE)
    }

    fn finish_debounce(&mut self) {
        let docs = std::mem::take(&mut self.docs);

        job::dispatch_blocking(move |editor, _compositor| {
            for doc in docs {
                request_blame(editor, doc);
            }
        });
    }
}

fn request_blame(editor: &mut Editor, doc_id: DocumentId) {
    if !editor.config().inline_blame.enable {
        return;
    }

    let Some(doc) = editor.document(doc_id) else {
        return;
    };
    let Some(path) = doc.path().map(Path::to_path_buf) else {
        return;
    };

    let trust_full = editor
        .workspace_trust
        .query(doc.workspace_root(), TrustQuery::Git)
        .is_trusted();
    let diff_providers = editor.diff_providers.clone();

    tokio::task::spawn_blocking(move || {
        let blame = diff_providers.blame_file(&path, trust_full);
        job::dispatch_blocking(move |editor, _compositor| {
            attach_blame(editor, doc_id, path, blame);
        });
    });
}

fn attach_blame(editor: &mut Editor, doc_id: DocumentId, path: PathBuf, blame: Option<FileBlame>) {
    let Some(doc) = editor.documents.get_mut(&doc_id) else {
        return;
    };
    if doc.path() == Some(path.as_path()) {
        doc.blame = blame;
    }
}

pub(super) fn register_hooks(handlers: &Handlers) {
    let tx = handlers.blame.clone();
    register_hook!(move |event: &mut DocumentDidOpen<'_>| {
        helix_event::send_blocking(&tx, BlameEvent(event.doc));
        Ok(())
    });

    register_hook!(move |event: &mut ConfigDidChange<'_>| {
        if event.new.inline_blame.enable && !event.old.inline_blame.enable {
            let docs: Vec<_> = event.editor.documents().map(|doc| doc.id()).collect();
            for doc in docs {
                request_blame(event.editor, doc);
            }
        }
        Ok(())
    });
}
