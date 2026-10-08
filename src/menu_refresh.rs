use block2::RcBlock;
use objc2::{
    rc::Retained,
    runtime::{NSObjectProtocol, ProtocolObject},
};
use objc2_foundation::{NSNotificationCenter, NSString};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

pub struct Tracking {
    center: Retained<NSNotificationCenter>,
    observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
    depth: Arc<AtomicUsize>,
}
fn change_depth(depth: &AtomicUsize, opening: bool) {
    let _ = depth.try_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
        Some(if opening {
            n.saturating_add(1)
        } else {
            n.saturating_sub(1)
        })
    });
}
impl Tracking {
    pub fn new() -> Self {
        let center = NSNotificationCenter::defaultCenter();
        let depth = Arc::new(AtomicUsize::new(0));
        let mut observers = Vec::new();
        for (name, opening) in [
            ("NSMenuDidBeginTrackingNotification", true),
            ("NSMenuDidEndTrackingNotification", false),
        ] {
            let depth = depth.clone();
            let block = RcBlock::new(move |_| change_depth(&depth, opening));
            // No object filter; receive synchronous callbacks for nested menus too.
            // The callback owns only an Arc<AtomicUsize>, so it is sendable.
            observers.push(unsafe {
                center.addObserverForName_object_queue_usingBlock(
                    Some(&NSString::from_str(name)),
                    None,
                    None,
                    &block,
                )
            });
        }
        Self {
            center,
            observers,
            depth,
        }
    }
    pub fn is_open(&self) -> bool {
        self.depth.load(Ordering::SeqCst) > 0
    }
}
impl Drop for Tracking {
    fn drop(&mut self) {
        for observer in &self.observers {
            unsafe {
                self.center.removeObserver((**observer).as_ref());
            }
        }
    }
}

#[derive(Default)]
pub struct Refresh {
    pub pending: bool,
}
impl Refresh {
    pub fn take(&mut self, changed: bool, can_replace: bool) -> bool {
        self.pending |= changed;
        if self.pending && can_replace {
            self.pending = false;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn updates_wait_for_tracking_to_end_and_coalesce() {
        let depth = AtomicUsize::new(0);
        change_depth(&depth, true);
        change_depth(&depth, true);
        change_depth(&depth, false);
        assert_eq!(depth.load(Ordering::SeqCst), 1);
        change_depth(&depth, false);
        change_depth(&depth, false);
        assert_eq!(depth.load(Ordering::SeqCst), 0);
        let mut refresh = Refresh::default();
        assert!(!refresh.take(true, false));
        assert!(!refresh.take(true, false));
        assert!(!refresh.take(false, false));
        assert!(refresh.pending);
        assert!(refresh.take(false, true));
        assert!(!refresh.take(false, true));
        assert!(refresh.take(true, true));
    }
}
