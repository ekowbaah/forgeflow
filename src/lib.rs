use std::collections::{HashMap, VecDeque};
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Queued,
    Running,
    Succeeded,
    Failed,
}
#[derive(Debug, Clone)]
pub struct Job {
    pub id: String,
    pub payload: String,
    pub state: State,
    pub attempts: u32,
    pub max_attempts: u32,
}
#[derive(Default)]
pub struct Engine {
    jobs: HashMap<String, Job>,
    queue: VecDeque<String>,
    audit: Vec<String>,
}
impl Engine {
    pub fn submit(
        &mut self,
        id: impl Into<String>,
        payload: impl Into<String>,
        max_attempts: u32,
    ) -> bool {
        let id = id.into();
        if self.jobs.contains_key(&id) {
            return false;
        }
        self.jobs.insert(
            id.clone(),
            Job {
                id: id.clone(),
                payload: payload.into(),
                state: State::Queued,
                attempts: 0,
                max_attempts: max_attempts.max(1),
            },
        );
        self.queue.push_back(id.clone());
        self.audit.push(format!("submitted:{id}"));
        true
    }
    pub fn claim(&mut self) -> Option<Job> {
        let id = self.queue.pop_front()?;
        let job = self.jobs.get_mut(&id)?;
        job.state = State::Running;
        job.attempts += 1;
        self.audit.push(format!("claimed:{id}:{}", job.attempts));
        Some(job.clone())
    }
    pub fn complete(&mut self, id: &str, success: bool) -> bool {
        let Some(job) = self.jobs.get_mut(id) else {
            return false;
        };
        if success {
            job.state = State::Succeeded
        } else if job.attempts < job.max_attempts {
            job.state = State::Queued;
            self.queue.push_back(id.into())
        } else {
            job.state = State::Failed
        };
        self.audit.push(format!("completed:{id}:{:?}", job.state));
        true
    }
    pub fn job(&self, id: &str) -> Option<&Job> {
        self.jobs.get(id)
    }
    pub fn audit(&self) -> &[String] {
        &self.audit
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idempotent_submit_and_retry() {
        let mut e = Engine::default();
        assert!(e.submit("j1", "{}", 2));
        assert!(!e.submit("j1", "other", 2));
        let j = e.claim().unwrap();
        e.complete(&j.id, false);
        assert_eq!(e.claim().unwrap().attempts, 2);
        e.complete("j1", true);
        assert_eq!(e.job("j1").unwrap().state, State::Succeeded);
        assert_eq!(e.audit().len(), 5);
    }
}
