use uuid::Uuid;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct TransferId(Uuid);

impl TransferId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

pub struct TaskBuildOptions {

}

