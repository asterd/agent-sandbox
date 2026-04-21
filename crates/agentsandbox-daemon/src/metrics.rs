//! In-process counters exposed in Prometheus text format.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub struct Metrics {
    pub sandboxes_created: AtomicU64,
    pub sandboxes_active: AtomicU64,
    pub sandboxes_expired: AtomicU64,
    pub exec_total: AtomicU64,
    pub egress_allowed: AtomicU64,
    pub egress_denied: AtomicU64,
    pub backend_errors: AtomicU64,
}

impl Metrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            sandboxes_created: AtomicU64::new(0),
            sandboxes_active: AtomicU64::new(0),
            sandboxes_expired: AtomicU64::new(0),
            exec_total: AtomicU64::new(0),
            egress_allowed: AtomicU64::new(0),
            egress_denied: AtomicU64::new(0),
            backend_errors: AtomicU64::new(0),
        })
    }

    pub fn sandbox_created(&self) {
        self.sandboxes_created.fetch_add(1, Ordering::Relaxed);
        self.sandboxes_active.fetch_add(1, Ordering::Relaxed);
    }

    pub fn sandbox_expired(&self, was_active: bool) {
        self.sandboxes_expired.fetch_add(1, Ordering::Relaxed);
        if was_active {
            self.decrement_active();
        }
    }

    pub fn sandbox_destroyed(&self, was_active: bool) {
        if was_active {
            self.decrement_active();
        }
    }

    pub fn exec_finished(&self) {
        self.exec_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn backend_error(&self) {
        self.backend_errors.fetch_add(1, Ordering::Relaxed);
    }

    fn decrement_active(&self) {
        let _ =
            self.sandboxes_active
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                    if current > 0 {
                        Some(current - 1)
                    } else {
                        None
                    }
                });
    }

    pub fn to_prometheus(&self) -> String {
        format!(
            "# HELP agentsandbox_sandboxes_created_total Sandbox create riuscite\n\
             # TYPE agentsandbox_sandboxes_created_total counter\n\
             agentsandbox_sandboxes_created_total {}\n\
             # HELP agentsandbox_sandboxes_active Sandbox attive\n\
             # TYPE agentsandbox_sandboxes_active gauge\n\
             agentsandbox_sandboxes_active {}\n\
             # HELP agentsandbox_sandboxes_expired_total Sandbox scadute per TTL\n\
             # TYPE agentsandbox_sandboxes_expired_total counter\n\
             agentsandbox_sandboxes_expired_total {}\n\
             # HELP agentsandbox_exec_total Exec completate\n\
             # TYPE agentsandbox_exec_total counter\n\
             agentsandbox_exec_total {}\n\
             # HELP agentsandbox_egress_allowed_total Connessioni egress consentite\n\
             # TYPE agentsandbox_egress_allowed_total counter\n\
             agentsandbox_egress_allowed_total {}\n\
             # HELP agentsandbox_egress_denied_total Connessioni egress negate\n\
             # TYPE agentsandbox_egress_denied_total counter\n\
             agentsandbox_egress_denied_total {}\n\
             # HELP agentsandbox_backend_errors_total Errori backend osservati dal daemon\n\
             # TYPE agentsandbox_backend_errors_total counter\n\
             agentsandbox_backend_errors_total {}\n",
            self.sandboxes_created.load(Ordering::Relaxed),
            self.sandboxes_active.load(Ordering::Relaxed),
            self.sandboxes_expired.load(Ordering::Relaxed),
            self.exec_total.load(Ordering::Relaxed),
            self.egress_allowed.load(Ordering::Relaxed),
            self.egress_denied.load(Ordering::Relaxed),
            self.backend_errors.load(Ordering::Relaxed),
        )
    }
}
