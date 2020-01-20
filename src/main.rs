#[macro_use]
extern crate common_failures;
#[macro_use]
extern crate failure;
#[macro_use]
extern crate log;
#[macro_use]
extern crate clap;
extern crate chrono;
extern crate redis;
extern crate stderrlog;

use common_failures::prelude::*;

use chrono::Local;
use chrono::NaiveTime;
use chrono::Timelike;
use clap::App;
use clap::Arg;
use redis::Commands;
use std::cmp::Ordering::Less;
use std::fmt;
use std::thread;
use std::time::Duration;
use std::vec::Vec;
use DayOrNight::Day;
use DayOrNight::Night;

const ARG_VERBOSITY: &str = "verbosity";
const ARG_QUIET: &str = "quiet";

const ARG_DAY: &str = "day";
const ARG_NIGHT: &str = "night";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum DayOrNight {
    Day,
    Night,
}

impl fmt::Display for DayOrNight {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct DayOrNightTrigger {
    start: NaiveTime,
    phase: DayOrNight,
}

struct DayNightCycle {
    phase_triggers: Vec<DayOrNightTrigger>,
    delta_to_zero: chrono::Duration,
}

impl DayNightCycle {
    fn new(day_start: NaiveTime, night_start: NaiveTime) -> DayNightCycle {
        let mut phase_triggers = Vec::new();
        phase_triggers.push(DayOrNightTrigger {
            start: day_start,
            phase: Day,
        });
        phase_triggers.push(DayOrNightTrigger {
            start: night_start,
            phase: Night,
        });

        phase_triggers.sort();

        let lowest_time = phase_triggers.get(0).unwrap().start.clone();

        let delta_to_zero = NaiveTime::from_hms(0, 0, 0) - lowest_time;

        for i in &mut phase_triggers {
            i.start += delta_to_zero;
        }

        DayNightCycle {
            phase_triggers,
            delta_to_zero,
        }
    }

    fn determine_current_phase(&self) -> DayOrNight {
        let now = Local::now().time() + self.delta_to_zero;
        let mut phase = None;
        for phase_trigger in &self.phase_triggers {
            if now.cmp(&phase_trigger.start) != Less {
                phase = Some(phase_trigger.phase);
            }
        }
        phase.unwrap()
    }
}

fn run() -> Result<()> {
    let args = App::new(crate_name!())
        .version(crate_version!())
        .author(crate_authors!())
        .arg(
            Arg::with_name(ARG_VERBOSITY)
                .long(ARG_VERBOSITY)
                .short("v")
                .multiple(true)
                .takes_value(false)
                .required(false),
        )
        .arg(
            Arg::with_name(ARG_QUIET)
                .long(ARG_QUIET)
                .short("q")
                .multiple(false)
                .takes_value(false)
                .required(false),
        )
        .arg(
            Arg::with_name(ARG_DAY)
                .long(ARG_DAY)
                .multiple(false)
                .takes_value(true)
                .required(false)
                .default_value("6:00"),
        )
        .arg(
            Arg::with_name(ARG_NIGHT)
                .long(ARG_NIGHT)
                .multiple(false)
                .takes_value(true)
                .required(false)
                .default_value("2:00"),
        )
        .get_matches();

    let verbosity = args.occurrences_of(ARG_VERBOSITY) as usize + 1;
    let quiet = args.is_present(ARG_QUIET);

    stderrlog::new()
        .module(module_path!())
        .timestamp(stderrlog::Timestamp::Second)
        .verbosity(verbosity)
        .quiet(quiet)
        .init()?;

    let day_start =
        NaiveTime::parse_from_str(value_t_or_exit!(args, ARG_DAY, String).as_str(), "%H:%M")?;

    let night_start =
        NaiveTime::parse_from_str(value_t_or_exit!(args, ARG_NIGHT, String).as_str(), "%H:%M")?;

    let day_night_cycle = DayNightCycle::new(day_start, night_start);
    let mut phase = day_night_cycle.determine_current_phase();

    let mut redis = redis::Client::open("redis://127.0.0.1/")?.get_connection()?;

    redis.hset_multiple(
        "day-night-cycle",
        &[
            ("start_time_day", day_start.format("%H:%M").to_string()),
            ("start_time_night", night_start.format("%H:%M").to_string()),
            ("current_phase", phase.to_string()),
        ],
    )?;

    redis.publish(
        "day-night-cycle/start_time_day",
        day_start.format("%H:%M").to_string(),
    )?;
    redis.publish(
        "day-night-cycle/start_time_night",
        night_start.format("%H:%M").to_string(),
    )?;
    redis.publish("day-night-cycle/current_phase", phase.to_string())?;

    info!("We start at {}", phase);

    loop {
        let now = Local::now().time();

        let nanos = now.second() as u64 * 1000000000 + now.nanosecond() as u64;

        // get delay till next full minute
        let delay: u64 = (1000000000 * 60) - nanos;
        thread::sleep(Duration::from_nanos(delay));

        let new_phase = day_night_cycle.determine_current_phase();
        if phase != new_phase {
            info!("{} changed to {}", phase, new_phase);
            phase = new_phase;
            redis.hset("day-night-cycle", "current_phase", phase.to_string())?;
            redis.publish("day-night-cycle/current_phase", phase.to_string())?;
        } else {
            debug!("It's still {}", phase);
        }
    }
}

quick_main!(run);
