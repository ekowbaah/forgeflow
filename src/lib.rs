use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
pub mod api;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum State {
    Scheduled,
    Queued,
    Running,
    Succeeded,
    Failed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub payload: String,
    pub state: State,
    pub attempts: u32,
    pub max_attempts: u32,
    pub available_at_ms: u64,
    pub lease_expires_at_ms: Option<u64>,
    pub claimed_by: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    pub job: Job,
    pub token: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Viewer,
    Worker,
    Operator,
    Admin,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Read,
    Claim,
    Complete,
    Submit,
    Cancel,
}
pub fn authorized(role: Role, action: Action) -> bool {
    match role {
        Role::Viewer => action == Action::Read,
        Role::Worker => matches!(action, Action::Read | Action::Claim | Action::Complete),
        Role::Operator => action != Action::Cancel,
        Role::Admin => true,
    }
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
        self.submit_at(id, payload, max_attempts, 0)
    }
    pub fn submit_at(
        &mut self,
        id: impl Into<String>,
        payload: impl Into<String>,
        max_attempts: u32,
        available_at_ms: u64,
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
                state: if available_at_ms == 0 {
                    State::Queued
                } else {
                    State::Scheduled
                },
                attempts: 0,
                max_attempts: max_attempts.max(1),
                available_at_ms,
                lease_expires_at_ms: None,
                claimed_by: None,
            },
        );
        if available_at_ms == 0 {
            self.queue.push_back(id.clone());
        }
        self.audit.push(format!("submitted:{id}:{available_at_ms}"));
        true
    }
    pub fn promote_due(&mut self, now_ms: u64) -> usize {
        let due: Vec<String> = self
            .jobs
            .values()
            .filter(|job| job.state == State::Scheduled && job.available_at_ms <= now_ms)
            .map(|job| job.id.clone())
            .collect();
        for id in &due {
            if let Some(job) = self.jobs.get_mut(id) {
                job.state = State::Queued;
                self.queue.push_back(id.clone());
                self.audit.push(format!("promoted:{id}"));
            }
        }
        due.len()
    }
    pub fn claim(&mut self) -> Option<Job> {
        self.claim_at("legacy-worker", 0, 30_000)
            .map(|claim| claim.job)
    }
    pub fn claim_at(&mut self, worker: &str, now_ms: u64, lease_ms: u64) -> Option<Claim> {
        let id = self.queue.pop_front()?;
        let job = self.jobs.get_mut(&id)?;
        job.state = State::Running;
        job.attempts += 1;
        job.claimed_by = Some(worker.to_owned());
        job.lease_expires_at_ms = Some(now_ms.saturating_add(lease_ms.max(1)));
        let token = format!("{id}:{worker}:{}", job.attempts);
        self.audit
            .push(format!("claimed:{id}:{worker}:{}", job.attempts));
        Some(Claim {
            job: job.clone(),
            token,
        })
    }
    pub fn complete(&mut self, id: &str, success: bool) -> bool {
        let Some(job) = self.jobs.get(id) else {
            return false;
        };
        let Some(worker) = job.claimed_by.as_deref() else {
            return false;
        };
        let token = format!("{id}:{worker}:{}", job.attempts);
        self.complete_claim(id, &token, success, 0)
    }
    pub fn complete_claim(
        &mut self,
        id: &str,
        token: &str,
        success: bool,
        retry_delay_ms: u64,
    ) -> bool {
        let Some(job) = self.jobs.get_mut(id) else {
            return false;
        };
        let expected = job
            .claimed_by
            .as_ref()
            .map(|worker| format!("{id}:{worker}:{}", job.attempts));
        if job.state != State::Running || expected.as_deref() != Some(token) {
            return false;
        }
        job.lease_expires_at_ms = None;
        job.claimed_by = None;
        if success {
            job.state = State::Succeeded
        } else if job.attempts < job.max_attempts {
            job.available_at_ms = retry_delay_ms;
            if retry_delay_ms == 0 {
                job.state = State::Queued;
                self.queue.push_back(id.into())
            } else {
                job.state = State::Scheduled
            }
        } else {
            job.state = State::Failed
        };
        self.audit.push(format!("completed:{id}:{:?}", job.state));
        true
    }
    pub fn heartbeat(&mut self, id: &str, token: &str, now_ms: u64, lease_ms: u64) -> bool {
        let Some(job) = self.jobs.get_mut(id) else {
            return false;
        };
        let expected = job
            .claimed_by
            .as_ref()
            .map(|worker| format!("{id}:{worker}:{}", job.attempts));
        if job.state != State::Running || expected.as_deref() != Some(token) {
            return false;
        }
        job.lease_expires_at_ms = Some(now_ms.saturating_add(lease_ms.max(1)));
        true
    }
    pub fn recover_expired(&mut self, now_ms: u64) -> usize {
        let expired: Vec<String> = self
            .jobs
            .values()
            .filter(|job| {
                job.state == State::Running
                    && job
                        .lease_expires_at_ms
                        .is_some_and(|expiry| expiry <= now_ms)
            })
            .map(|job| job.id.clone())
            .collect();
        for id in &expired {
            let job = self.jobs.get_mut(id).expect("selected job exists");
            job.claimed_by = None;
            job.lease_expires_at_ms = None;
            if job.attempts < job.max_attempts {
                job.state = State::Queued;
                self.queue.push_back(id.clone())
            } else {
                job.state = State::Failed
            }
            self.audit.push(format!("lease-expired:{id}"));
        }
        expired.len()
    }
    pub fn job(&self, id: &str) -> Option<&Job> {
        self.jobs.get(id)
    }
    pub fn audit(&self) -> &[String] {
        &self.audit
    }
    pub fn jobs(&self) -> Vec<Job> {
        let mut jobs: Vec<_> = self.jobs.values().cloned().collect();
        jobs.sort_by(|a, b| a.id.cmp(&b.id));
        jobs
    }
    pub fn cancel(&mut self, id: &str) -> bool {
        let Some(job) = self.jobs.get_mut(id) else {
            return false;
        };
        if matches!(job.state, State::Succeeded | State::Failed) {
            return false;
        }
        job.state = State::Failed;
        job.claimed_by = None;
        job.lease_expires_at_ms = None;
        self.queue.retain(|queued| queued != id);
        self.audit.push(format!("cancelled:{id}"));
        true
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
    #[test]
    fn schedules_and_promotes_jobs_deterministically() {
        let mut e = Engine::default();
        assert!(e.submit_at("later", "{}", 1, 500));
        assert!(e.claim_at("w", 499, 10).is_none());
        assert_eq!(e.promote_due(499), 0);
        assert_eq!(e.promote_due(500), 1);
        assert_eq!(e.claim_at("w", 500, 10).unwrap().job.id, "later");
    }
    #[test]
    fn lease_recovery_requeues_with_a_new_claim_token() {
        let mut e = Engine::default();
        e.submit("j", "{}", 2);
        let first = e.claim_at("a", 100, 20).unwrap();
        assert!(!e.complete_claim("j", "wrong", true, 0));
        assert_eq!(e.recover_expired(119), 0);
        assert_eq!(e.recover_expired(120), 1);
        let second = e.claim_at("b", 121, 20).unwrap();
        assert_ne!(first.token, second.token);
        assert!(e.complete_claim("j", &second.token, true, 0));
    }
    #[test]
    fn heartbeats_extend_leases() {
        let mut e = Engine::default();
        e.submit("j", "{}", 1);
        let claim = e.claim_at("a", 0, 10).unwrap();
        assert!(e.heartbeat("j", &claim.token, 9, 10));
        assert_eq!(e.recover_expired(10), 0);
        assert_eq!(e.recover_expired(19), 1);
        assert_eq!(e.job("j").unwrap().state, State::Failed);
    }
    #[test]
    fn roles_are_least_privilege() {
        assert!(authorized(Role::Viewer, Action::Read));
        assert!(!authorized(Role::Viewer, Action::Submit));
        assert!(authorized(Role::Worker, Action::Complete));
        assert!(!authorized(Role::Operator, Action::Cancel));
        assert!(authorized(Role::Admin, Action::Cancel));
    }
}
