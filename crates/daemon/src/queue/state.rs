//! One lane's bookkeeping as plain data. No runtime, no I/O: a test can drive it directly
//! (§12 rule 10).

use shimmer_core::{Error, LaneConfig, Priority, Result, Task, TaskId};

pub struct LaneState {
    pub cfg: LaneConfig,
    /// Waiting tasks, next-up first.
    pub queued: Vec<Task>,
    pub running: Vec<TaskId>,
    /// Bumped on every mutation of this lane; the optimistic-concurrency token clients echo.
    pub version: u64,
}

impl LaneState {
    pub fn new(cfg: LaneConfig) -> Self {
        Self { cfg, queued: Vec::new(), running: Vec::new(), version: 0 }
    }

    /// Insert and return the number of tasks ahead of it. Everything is FIFO, except that
    /// an explicitly promoted task goes ahead of all non-promoted ones (behind earlier
    /// promotions). `Scheduled` and `Normal` keep arrival order: a manual task takes its
    /// normal turn (§6.2).
    pub fn enqueue(&mut self, task: Task) -> usize {
        let pos = if matches!(task.priority, Priority::Overridden { .. }) {
            self.queued
                .iter()
                .position(|t| !matches!(t.priority, Priority::Overridden { .. }))
                .unwrap_or(self.queued.len())
        } else {
            self.queued.len()
        };
        self.queued.insert(pos, task);
        self.version += 1;
        pos
    }

    /// Move waiting tasks into `running` while there is capacity. The running task is never
    /// displaced: only free slots are filled.
    pub fn take_startable(&mut self) -> Vec<Task> {
        let free = self.cfg.max_concurrent.saturating_sub(self.running.len());
        let n = free.min(self.queued.len());
        if n == 0 {
            return Vec::new();
        }
        let started: Vec<Task> = self.queued.drain(..n).collect();
        self.running.extend(started.iter().map(|t| t.id));
        self.version += 1;
        started
    }

    /// Free the slot. Called for every outcome; a failure never holds a lane (§11.1).
    pub fn finish(&mut self, id: TaskId) {
        self.running.retain(|r| *r != id);
        self.version += 1;
    }

    pub fn remove_queued(&mut self, id: TaskId) -> Option<Task> {
        let i = self.queued.iter().position(|t| t.id == id)?;
        self.version += 1;
        Some(self.queued.remove(i))
    }

    /// Waiting tasks an `Overridden` arrival would jump: every non-overridden one. Earlier
    /// overrides are not displaced (they keep their turn), and a running task never is. Empty
    /// means promotion costs nobody anything, so it needs no confirmation.
    pub fn displaced_by_override(&self) -> Vec<&Task> {
        self.queued.iter().filter(|t| !is_overridden(t)).collect()
    }

    /// Move a waiting task to sit just before `before`, or to the back when `None`. Returns
    /// its new index. Nothing changes on error, and `version` moves only if the order did.
    ///
    /// Only waiting tasks are movable (the running one cannot be reordered, §6.2), and the
    /// overridden block stays ahead of everything else: reordering is not a second, unaudited
    /// way to promote.
    // Wired up by `queue.reorder`, which waits on a `proto` change (docs/decisions/0006).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn reorder(&mut self, id: TaskId, before: Option<TaskId>) -> Result<usize> {
        let from = self
            .queued
            .iter()
            .position(|t| t.id == id)
            .ok_or_else(|| Error::not_found(format!("task {id} is not waiting in lane '{}'", self.cfg.id)))?;
        if before == Some(id) {
            return Err(Error::invalid_params("a task cannot be placed before itself"));
        }
        let mut order = self.queued.clone();
        let task = order.remove(from);
        let to = match before {
            None => order.len(),
            Some(b) => order
                .iter()
                .position(|t| t.id == b)
                .ok_or_else(|| Error::not_found(format!("task {b} is not waiting in lane '{}'", self.cfg.id)))?,
        };
        let boundary = order.iter().take_while(|t| is_overridden(t)).count();
        let allowed = if is_overridden(&task) { to <= boundary } else { to >= boundary };
        if !allowed {
            return Err(Error::invalid_params("reorder cannot move a task across the promoted block"));
        }
        order.insert(to, task);
        if to != from {
            self.queued = order;
            self.version += 1;
        }
        Ok(to)
    }
}

fn is_overridden(t: &Task) -> bool {
    matches!(t.priority, Priority::Overridden { .. })
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;
    use shimmer_core::Origin;

    use super::*;

    fn task(priority: Priority) -> Task {
        Task {
            id: TaskId::new(),
            lane: "l".into(),
            op: "m.op".into(),
            params: json!(null),
            priority,
            origin: Origin::User,
            fallback: None,
            enqueued_at: Utc::now(),
        }
    }

    fn promoted() -> Priority {
        Priority::Overridden { by: "user".into(), at: Utc::now() }
    }

    #[test]
    fn respects_max_concurrent_and_fifo() {
        let mut l = LaneState::new(LaneConfig::new("l", 2));
        let ts: Vec<_> = (0..4).map(|_| task(Priority::Normal)).collect();
        let ids: Vec<_> = ts.iter().map(|t| t.id).collect();
        let positions: Vec<_> = ts.into_iter().map(|t| l.enqueue(t)).collect();
        assert_eq!(positions, [0, 1, 2, 3]);

        let started: Vec<_> = l.take_startable().iter().map(|t| t.id).collect();
        assert_eq!(started, ids[..2]);
        assert!(l.take_startable().is_empty(), "lane is full");

        l.finish(ids[0]);
        assert_eq!(l.take_startable()[0].id, ids[2]);
    }

    #[test]
    fn promotion_jumps_normal_but_not_earlier_promotions() {
        let mut l = LaneState::new(LaneConfig::new("l", 1));
        let a = task(Priority::Scheduled);
        let b = task(Priority::Normal);
        let p1 = task(promoted());
        let p2 = task(promoted());
        let ids = [p1.id, p2.id, a.id, b.id];
        l.enqueue(a);
        l.enqueue(b);
        assert_eq!(l.enqueue(p1), 0);
        assert_eq!(l.enqueue(p2), 1);
        assert_eq!(l.queued.iter().map(|t| t.id).collect::<Vec<_>>(), ids);
    }

    fn ids(l: &LaneState) -> Vec<TaskId> {
        l.queued.iter().map(|t| t.id).collect()
    }

    /// §6.2: priority records where a task came from, not a rank. Scheduled and Normal share
    /// one arrival-order queue, in either direction.
    #[test]
    fn scheduled_and_normal_share_one_arrival_order() {
        let mut l = LaneState::new(LaneConfig::new("l", 1));
        let tasks: Vec<_> = [
            Priority::Normal,
            Priority::Scheduled,
            Priority::Scheduled,
            Priority::Normal,
            Priority::Scheduled,
            Priority::Normal,
        ]
        .into_iter()
        .map(task)
        .collect();
        let arrival: Vec<_> = tasks.iter().map(|t| t.id).collect();
        for (i, t) in tasks.into_iter().enumerate() {
            assert_eq!(l.enqueue(t), i, "always joins at the back");
        }
        assert_eq!(ids(&l), arrival);

        let mut started = Vec::new();
        while let Some(t) = l.take_startable().pop() {
            started.push(t.id);
            l.finish(t.id);
        }
        assert_eq!(started, arrival, "runs in the order enqueued, whatever the tier");
    }

    #[test]
    fn override_displaces_only_waiting_non_overridden_tasks() {
        let mut l = LaneState::new(LaneConfig::new("l", 1));
        assert!(l.displaced_by_override().is_empty(), "empty lane: nobody to displace");

        let (running, a, b, p) =
            (task(Priority::Normal), task(Priority::Scheduled), task(Priority::Normal), task(promoted()));
        let (aid, bid, pid) = (a.id, b.id, p.id);
        for t in [running, a, b] {
            l.enqueue(t);
        }
        l.take_startable();
        let displaced: Vec<_> = l.displaced_by_override().iter().map(|t| t.id).collect();
        assert_eq!(displaced, [aid, bid], "the running task is not displaced");

        l.enqueue(p);
        let displaced: Vec<_> = l.displaced_by_override().iter().map(|t| t.id).collect();
        assert_eq!(displaced, [aid, bid], "an earlier override keeps its turn and is not listed");
        assert_eq!(ids(&l)[0], pid);
    }

    #[test]
    fn reorder_moves_within_the_waiting_list_only() {
        let mut l = LaneState::new(LaneConfig::new("l", 1));
        let ts: Vec<_> = (0..4).map(|_| task(Priority::Normal)).collect();
        let [r, a, b, c] = [ts[0].id, ts[1].id, ts[2].id, ts[3].id];
        for t in ts {
            l.enqueue(t);
        }
        l.take_startable(); // r is now running
        let v = l.version;

        assert_eq!(l.reorder(c, Some(a)).unwrap(), 0);
        assert_eq!(ids(&l), [c, a, b]);
        assert!(l.version > v);

        assert_eq!(l.reorder(c, None).unwrap(), 2);
        assert_eq!(ids(&l), [a, b, c]);

        let v = l.version;
        assert_eq!(l.reorder(a, Some(b)).unwrap(), 0, "already there");
        assert_eq!(l.version, v, "a no-op does not bump the version");

        for bad in [l.reorder(r, None), l.reorder(a, Some(r)), l.reorder(a, Some(a))] {
            assert!(bad.is_err(), "running task is neither movable nor a target; self-target is nonsense");
        }
        assert_eq!(ids(&l), [a, b, c], "errors change nothing");
        assert_eq!(l.version, v);
    }

    #[test]
    fn reorder_cannot_smuggle_a_promotion() {
        let mut l = LaneState::new(LaneConfig::new("l", 1));
        let (running, a, b, p1, p2) = (
            task(Priority::Normal),
            task(Priority::Normal),
            task(Priority::Scheduled),
            task(promoted()),
            task(promoted()),
        );
        let [aid, bid, p1id, p2id] = [a.id, b.id, p1.id, p2.id];
        for t in [running, a, b] {
            l.enqueue(t);
        }
        l.take_startable();
        l.enqueue(p1);
        l.enqueue(p2);
        assert_eq!(ids(&l), [p1id, p2id, aid, bid]);

        assert!(l.reorder(bid, Some(p1id)).is_err(), "unpromoted task cannot jump the promoted block");
        assert!(l.reorder(aid, Some(p2id)).is_err());
        assert!(l.reorder(p1id, None).is_err(), "nor can a promoted task fall behind ordinary ones");
        assert_eq!(l.reorder(p2id, Some(p1id)).unwrap(), 0, "reordering within the block is fine");
        assert_eq!(l.reorder(bid, Some(aid)).unwrap(), 2, "as is within the ordinary tasks");
        assert_eq!(ids(&l), [p2id, p1id, bid, aid]);
    }

    #[test]
    fn failure_frees_the_slot_and_versions_bump() {
        let mut l = LaneState::new(LaneConfig::new("l", 1));
        let (a, b) = (task(Priority::Normal), task(Priority::Normal));
        let (aid, bid) = (a.id, b.id);
        l.enqueue(a);
        l.enqueue(b);
        l.take_startable();
        let v = l.version;
        l.finish(aid);
        assert!(l.version > v);
        assert_eq!(l.take_startable()[0].id, bid);
        assert!(l.remove_queued(aid).is_none());
    }
}
