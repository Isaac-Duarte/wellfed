//! The background jobs' last runs: one row per job, replaced by each run.

use wellfed_api::{Job, JobOutcome, JobRun};

use crate::convert::{millis, timestamp};
use crate::{Result, Store};

impl Store {
    /// Keep `run` as its job's last run.
    pub async fn record_job_run(&self, run: &JobRun) -> Result<()> {
        let job = run.job.code();
        let started = millis(run.started);
        let finished = millis(run.finished);
        let outcome = run.outcome.code();
        sqlx::query!(
            "INSERT INTO job_runs (job, started_ms, finished_ms, outcome) VALUES (?, ?, ?, ?)
             ON CONFLICT (job) DO UPDATE SET
                 started_ms = excluded.started_ms,
                 finished_ms = excluded.finished_ms,
                 outcome = excluded.outcome",
            job,
            started,
            finished,
            outcome,
        )
        .execute(&self.writer)
        .await?;
        Ok(())
    }

    /// Every job's last run, in [`Job::ALL`]'s order; a job that has never
    /// run has none. A row a newer server wrote, for a job or an outcome
    /// this build cannot name, is left out: this build does not run that
    /// job, and failing the read would hide every other job's last run.
    pub async fn job_runs(&self) -> Result<Vec<JobRun>> {
        let rows = sqlx::query!("SELECT job, started_ms, finished_ms, outcome FROM job_runs")
            .fetch_all(&self.reader)
            .await?;

        let mut runs = Vec::new();
        for row in rows {
            let (Some(job), Some(outcome)) = (
                Job::from_code(&row.job),
                JobOutcome::from_code(&row.outcome),
            ) else {
                continue;
            };
            runs.push(JobRun {
                job,
                started: timestamp(row.started_ms)?,
                finished: timestamp(row.finished_ms)?,
                outcome,
            });
        }
        runs.sort_by_key(|run| Job::ALL.iter().position(|job| *job == run.job));
        Ok(runs)
    }
}
