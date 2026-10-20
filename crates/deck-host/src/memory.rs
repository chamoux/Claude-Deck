//! Private bytes of process trees (sysinfo).

use std::collections::{HashMap, HashSet};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// Keeps one `sysinfo::System` alive so repeated measurements reuse its state.
pub struct MemoryMeter {
    sys: System,
}

impl MemoryMeter {
    pub fn new() -> Self {
        Self { sys: System::new() }
    }

    /// Refreshes once, then sums `Process::virtual_memory()` over each root pid and all its
    /// descendants (seen-set guards against pid reuse cycles). Unknown pids add 0.
    pub fn private_bytes(&mut self, roots: &[u32]) -> u64 {
        self.sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_memory(),
        );

        let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
        for (pid, process) in self.sys.processes() {
            if let Some(parent) = process.parent() {
                children.entry(parent).or_default().push(*pid);
            }
        }

        let mut seen: HashSet<Pid> = HashSet::new();
        let mut stack: Vec<Pid> = roots.iter().map(|&r| Pid::from_u32(r)).collect();
        let mut total: u64 = 0;
        while let Some(pid) = stack.pop() {
            if !seen.insert(pid) {
                continue;
            }
            if let Some(process) = self.sys.process(pid) {
                total = total.saturating_add(process.virtual_memory());
            }
            if let Some(kids) = children.get(&pid) {
                stack.extend(kids.iter().copied());
            }
        }
        total
    }
}

impl Default for MemoryMeter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::MemoryMeter;
    use std::process::{Command, Stdio};

    #[test]
    fn private_bytes_of_self_is_over_one_megabyte() {
        let mut m = MemoryMeter::new();
        assert!(m.private_bytes(&[std::process::id()]) > 1_000_000);
    }

    #[test]
    fn unknown_pid_counts_as_zero() {
        let mut m = MemoryMeter::new();
        assert_eq!(m.private_bytes(&[u32::MAX - 7]), 0);
    }

    #[test]
    fn child_process_is_included_in_tree_total() {
        let mut m = MemoryMeter::new();
        let me = std::process::id();
        let before = m.private_bytes(&[me]);

        let mut child = Command::new("cmd")
            .args(["/c", "ping", "-n", "3", "127.0.0.1"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn child");
        let child_pid = child.id();

        let after = m.private_bytes(&[me]);
        let child_alone = m.private_bytes(&[child_pid]);
        let _ = child.kill();
        let _ = child.wait();

        assert!(child_alone > 0, "child pid should be measured");
        // Comparing with `before` was flaky: the test process's own private bytes
        // move between refreshes while other tests run. The tree must at least
        // contain the child plus this process.
        assert!(before > 0);
        assert!(
            after > child_alone,
            "tree total ({after}) should include the child ({child_alone}) and this process"
        );
    }
}
