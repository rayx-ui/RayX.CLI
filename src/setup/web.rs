//! The web set, ported for NSH in sprint 3; until then it plans no steps.

use super::{PlanEnv, SetupError, Step};

/// The steps of the set: none in this build yet.
pub fn steps(_env: &PlanEnv) -> Result<Vec<Step>, SetupError> {
    Ok(Vec::new())
}
