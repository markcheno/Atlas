//! Duty cycles for background jobs that do their own work in steps (text
//! extraction, embedding): after each step a job rests in proportion to
//! how long the step kept it busy, so it uses about `duty` of the time.

use std::time::Duration;

/// The shortest rest the duty cycle takes: shorter rests are added up
/// first. The busy fraction is the same; work comes in bursts of about
/// `REST_QUANTUM × duty / (1 − duty)` (100 ms at the Run duty cycle).
pub const REST_QUANTUM: Duration = Duration::from_millis(100);

/// Adds the rest work of `busy` owes at `duty` to `owed`, and returns
/// the rest to take now, if `owed` has reached [`REST_QUANTUM`].
pub fn rest_due(owed: &mut Duration, busy: Duration, duty: f64) -> Option<Duration> {
    if duty >= 1.0 {
        *owed = Duration::ZERO;
        return None;
    }
    *owed += busy.mul_f64((1.0 / duty.max(0.01)) - 1.0);
    (*owed >= REST_QUANTUM).then(|| std::mem::take(owed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_rests_add_up_before_they_are_taken() {
        let ms = Duration::from_millis;
        let mut owed = Duration::ZERO;
        // At the Run duty cycle each file owes its own length.
        for _ in 0..9 {
            assert_eq!(rest_due(&mut owed, ms(10), 0.5), None);
        }
        assert_eq!(rest_due(&mut owed, ms(10), 0.5), Some(ms(100)));
        assert_eq!(owed, Duration::ZERO);
        // A long file rests at once, as long as it owes.
        assert_eq!(rest_due(&mut owed, ms(300), 0.5), Some(ms(300)));
        // At the Slow duty cycle a file owes four times its length.
        assert_eq!(rest_due(&mut owed, ms(30), 0.2), Some(ms(120)));
        // Unthrottled never rests and forgets what was owed.
        assert_eq!(rest_due(&mut owed, ms(40), 0.5), None);
        assert_eq!(rest_due(&mut owed, ms(40), 1.0), None);
        assert_eq!(owed, Duration::ZERO);
    }
}
