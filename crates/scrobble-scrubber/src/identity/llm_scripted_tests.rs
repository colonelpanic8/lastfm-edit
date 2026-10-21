use super::{ChoiceRequest, LlmBackend, RawChoice, RawResearch, ResearchRequest};
use crate::identity::adjudicate::AdjudicateError;
use std::sync::{Arc, Mutex};

/// Replays canned answers in order; records every request.
#[derive(Default)]
pub(crate) struct ScriptedBackend {
    pub answers: Mutex<Vec<RawChoice>>,
    pub requests: Mutex<Vec<ChoiceRequest>>,
    pub research_answers: Mutex<Vec<RawResearch>>,
    pub research_requests: Mutex<Vec<ResearchRequest>>,
}

fn next<T>(script: &Mutex<Vec<T>>) -> Result<T, AdjudicateError> {
    let mut script = script.lock().unwrap();
    if script.is_empty() {
        return Err(AdjudicateError::Transport("script exhausted".into()));
    }
    Ok(script.remove(0))
}

#[async_trait::async_trait]
impl LlmBackend for ScriptedBackend {
    async fn choose(&self, request: ChoiceRequest) -> Result<RawChoice, AdjudicateError> {
        self.requests.lock().unwrap().push(request);
        next(&self.answers)
    }

    async fn research(&self, request: ResearchRequest) -> Result<RawResearch, AdjudicateError> {
        self.research_requests.lock().unwrap().push(request);
        next(&self.research_answers)
    }

    fn label(&self) -> String {
        "scripted".into()
    }
}

/// Lets a test keep a handle on the recorded requests after boxing the backend.
#[async_trait::async_trait]
impl LlmBackend for Arc<ScriptedBackend> {
    async fn choose(&self, request: ChoiceRequest) -> Result<RawChoice, AdjudicateError> {
        self.as_ref().choose(request).await
    }

    async fn research(&self, request: ResearchRequest) -> Result<RawResearch, AdjudicateError> {
        self.as_ref().research(request).await
    }

    fn label(&self) -> String {
        self.as_ref().label()
    }
}
