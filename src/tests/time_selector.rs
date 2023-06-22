use opening_hours_syntax::rules::RuleKind::*;

use crate::error::Result;
use crate::schedule_at;

#[test]
fn basic_timespan() -> Result<()> {
    assert_eq!(
        schedule_at!("14:00-19:00", "2020-06-01"),
        schedule! { 14,00 => Open => 19,00 }
    );

    assert_eq!(
        schedule_at!("10:00-12:00,14:00-16:00", "2020-06-01"),
        schedule! {
            10,00 => Open => 12,00;
            14,00 => Open => 16,00;
        }
    );

    assert_eq!(
        schedule_at!("10:00-12:00,11:00-16:00 unknown", "2020-06-01"),
        schedule! { 10,00 => Unknown => 16,00 }
    );

    Ok(())
}

#[test]
fn events() -> Result<()> {
    assert_eq!(
        schedule_at!("(dawn-02:30)-(dusk+02:30)", "2020-06-01"),
        schedule! { 3,30 => Open => 22,30 }
    );

    assert_eq!(
        schedule_at!("(dawn+00:30)-(dusk-00:30)", "2020-06-01"),
        schedule! { 6,30 => Open => 19,30 }
    );

    assert_eq!(
        schedule_at!("sunrise-19:45", "2020-06-01"),
        schedule! { 7,00 => Open => 19,45 }
    );

    assert_eq!(
        schedule_at!("08:15-sunset", "2020-06-01"),
        schedule! { 8,15 => Open => 19,00 }
    );

    Ok(())
}

#[test]
fn overlap() -> Result<()> {
    assert_eq!(
        schedule_at!("10:00-12:00,14:00-25:30", "2020-06-01"),
        schedule! {
            00,00 => Open =>  1,30;
            10,00 => Open => 12,00;
            14,00 => Open => 24,00;
        }
    );

    assert_eq!(
        schedule_at!("Mo 14:00-25:30", "2020-06-02"),
        schedule! { 00,00 => Open =>  1,30 }
    );

    Ok(())
}
