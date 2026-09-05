use anyhow::{Result, bail};
use std::{
    future::Future,
    time::{Duration, Instant},
};

pub(crate) const DEFAULT_PUBLIC_WORK_BUDGET: Duration = Duration::from_secs(240);
pub(crate) const DEFAULT_OPERATOR_WORK_BUDGET: Duration = Duration::from_secs(600);
pub(crate) const DETERMINISTIC_FALLBACK_RESERVE: Duration = Duration::from_secs(1);
pub(crate) const LOCAL_MODEL_PHASE_LIMIT: Duration = Duration::from_secs(90);
pub(crate) const DEFAULT_OPERATOR_CONTINUATION_RESERVE: Duration = Duration::from_secs(91);
pub(crate) const OPERATOR_MODEL_TOOL_PHASE_LIMIT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum InferenceLane {
    Public,
    Operator,
}

impl InferenceLane {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Operator => "operator",
        }
    }

    const fn default_budget(self) -> Duration {
        match self {
            Self::Public => DEFAULT_PUBLIC_WORK_BUDGET,
            Self::Operator => DEFAULT_OPERATOR_WORK_BUDGET,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct InferenceDeadline {
    lane: InferenceLane,
    expires_at: Instant,
}

tokio::task_local! {
    static AUTHENTICATED_DEADLINE: InferenceDeadline;
}

impl InferenceDeadline {
    fn after(lane: InferenceLane, budget: Duration) -> Result<Self> {
        if budget.is_zero() {
            bail!("inference deadline budget must be greater than zero");
        }
        let expires_at = Instant::now().checked_add(budget).ok_or_else(|| {
            anyhow::anyhow!("inference deadline exceeds the monotonic clock range")
        })?;
        Ok(Self { lane, expires_at })
    }

    pub(crate) fn current(lane: InferenceLane) -> Result<Self> {
        match AUTHENTICATED_DEADLINE.try_with(|deadline| *deadline) {
            Ok(deadline) if deadline.lane == lane => Ok(deadline),
            Ok(deadline) => bail!(
                "authenticated {} inference deadline cannot be used for the {} lane",
                deadline.lane.as_str(),
                lane.as_str()
            ),
            // Direct model/unit-test callers and the deliberately public stdin harness do not have
            // an XMTP envelope. They still receive the same bounded lane defaults.
            Err(_) => Self::after(lane, lane.default_budget()),
        }
    }

    pub(crate) const fn lane(self) -> InferenceLane {
        self.lane
    }

    pub(crate) fn remaining(self) -> Duration {
        self.expires_at.saturating_duration_since(Instant::now())
    }

    pub(crate) fn capped(self, maximum: Duration) -> Result<Self> {
        if maximum.is_zero() {
            bail!("inference deadline cap must be greater than zero");
        }
        let capped_at = Instant::now().checked_add(maximum).ok_or_else(|| {
            anyhow::anyhow!("inference deadline cap exceeds the monotonic clock range")
        })?;
        Ok(Self {
            lane: self.lane,
            expires_at: self.expires_at.min(capped_at),
        })
    }
}

pub(crate) async fn scope_authenticated_deadline<F>(
    lane: InferenceLane,
    budget: Duration,
    future: F,
) -> Result<F::Output>
where
    F: Future,
{
    let deadline = InferenceDeadline::after(lane, budget)?;
    Ok(AUTHENTICATED_DEADLINE.scope(deadline, future).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn authenticated_deadlines_are_lane_bound() {
        let result =
            scope_authenticated_deadline(InferenceLane::Public, Duration::from_secs(10), async {
                assert!(InferenceDeadline::current(InferenceLane::Public).is_ok());
                InferenceDeadline::current(InferenceLane::Operator)
            })
            .await
            .unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn callers_without_an_xmtp_scope_remain_bounded() {
        let public = InferenceDeadline::current(InferenceLane::Public).unwrap();
        let operator = InferenceDeadline::current(InferenceLane::Operator).unwrap();
        assert!(public.remaining() <= DEFAULT_PUBLIC_WORK_BUDGET);
        assert!(operator.remaining() <= DEFAULT_OPERATOR_WORK_BUDGET);
    }
}
