use super::*;

#[cfg(test)]
mod tests {
    use crate::core::types::{Priority, TransferId};
    use super::task::TransferTask;
    use super::types:: TransferState;

    #[test]
    fn test_transfer_state_transition() {
        use TransferState::*;

        // valid
        assert!(TransferTask::is_valid_transition(Queued, Preparing));
        assert!(TransferTask::is_valid_transition(Preparing, Transferring));
        assert!(TransferTask::is_valid_transition(Transferring, Completed));
        assert!(TransferTask::is_valid_transition(Transferring, Paused));
        assert!(TransferTask::is_valid_transition(Paused, Transferring));
        assert!(TransferTask::is_valid_transition(Failed, Queued));

        // invalid
        assert!(!TransferTask::is_valid_transition(Completed, Transferring));
        assert!(!TransferTask::is_valid_transition(Failed, Transferring));
        assert!(!TransferTask::is_valid_transition(Failed, Preparing));
        assert!(!TransferTask::is_valid_transition(Cancelled, Preparing));
    }

    #[test]
    fn test_transfer_id_generation() {
        let id1 = TransferId::new();
        let id2 = TransferId::new();

        assert_ne!(id1, id2);
        assert_eq!(id1, id1);

        let id_str = id1.to_string();
        assert!(!id_str.is_empty());
    }

    #[test]
    fn test_priority_ordering() {
        assert!(Priority::HIGH > Priority::NORMAL);
        assert!(Priority::NORMAL > Priority::LOW);
        assert_eq!(Priority::default(), Priority::NORMAL);
    }
}