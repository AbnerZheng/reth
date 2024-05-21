use crate::{
    commands::db::get::{maybe_json_value_parser, table_key},
    utils::DbTool,
};
use ahash::RandomState;
use clap::Parser;
use reth_db::{
    cursor::DbCursorRO, database::Database, table::Table, transaction::DbTx, DatabaseEnv, RawKey,
    RawTable, RawValue, TableViewer, Tables,
};
use std::{
    hash::{BuildHasher, Hasher},
    time::{Duration, Instant},
};
use tracing::{info, warn};

#[derive(Parser, Debug)]
/// The arguments for the `reth db checksum` command
pub struct Command {
    /// The table name
    table: Tables,

    /// The start of the range to checksum.
    #[arg(long, value_parser = maybe_json_value_parser)]
    start_key: Option<String>,

    /// The end of the range to checksum.
    #[arg(long, value_parser = maybe_json_value_parser)]
    end_key: Option<String>,

    /// The maximum number of records that are queried and used to compute the
    /// checksum.
    #[arg(long)]
    limit: Option<usize>,
}

impl Command {
    /// Execute `db checksum` command
    pub fn execute(self, tool: &DbTool<DatabaseEnv>) -> eyre::Result<()> {
        warn!("This command should be run without the node running!");
        self.table.view(&ChecksumViewer {
            tool,
            start_key: self.start_key,
            end_key: self.end_key,
            limit: self.limit,
        })
    }
}

pub(crate) struct ChecksumViewer<'a, DB: Database> {
    tool: &'a DbTool<DB>,
    start_key: Option<String>,
    end_key: Option<String>,
    limit: Option<usize>,
}

impl<DB: Database> ChecksumViewer<'_, DB> {
    pub(crate) fn new(tool: &'_ DbTool<DB>) -> ChecksumViewer<'_, DB> {
        ChecksumViewer { tool, start_key: None, end_key: None, limit: None }
    }

    pub(crate) fn get_checksum<T: Table>(&self) -> Result<(u64, Duration), eyre::Report> {
        let provider =
            self.tool.provider_factory.provider()?.disable_long_read_transaction_safety();
        let tx = provider.tx_ref();

        let mut cursor = tx.cursor_read::<RawTable<T>>()?;
        let walker = match (self.start_key.as_deref(), self.end_key.as_deref()) {
            (Some(start), Some(end)) => {
                info!("start={start} \n end={end}");
                let start_key = table_key::<T>(start).map(RawKey::<T::Key>::new)?;
                let end_key = table_key::<T>(end).map(RawKey::<T::Key>::new)?;
                cursor.walk_range(start_key..=end_key)?
            }
            (None, Some(end)) => {
                info!("start=.. \n end={end}");
                let end_key = table_key::<T>(end).map(RawKey::<T::Key>::new)?;

                cursor.walk_range(..=end_key)?
            }
            (Some(start), None) => {
                info!("start={start} \n end= ");
                let start_key = table_key::<T>(start).map(RawKey::<T::Key>::new)?;
                cursor.walk_range(start_key..)?
            }
            (None, None) => cursor.walk_range(..)?,
        };

        let start_time = Instant::now();
        let mut hasher = RandomState::with_seeds(1, 2, 3, 4).build_hasher();
        let mut total = 0;

        let limit = self.limit.unwrap_or(usize::MAX);
        let mut enumerate_start_key = None;
        let mut enumerate_end_key = None;
        for (index, entry) in walker.enumerate() {
            let (k, v): (RawKey<T::Key>, RawValue<T::Value>) = entry?;

            if index % 100_000 == 0 {
                info!("Hashed {index} entries.");
            }

            hasher.write(k.raw_key());
            hasher.write(v.raw_value());

            if enumerate_start_key.is_none() {
                enumerate_start_key = Some(k.clone());
            }
            enumerate_end_key = Some(k);

            total = index + 1;
            if total >= limit {
                break
            }
        }

        info!("Hashed {total} entries.");
        if let (Some(s), Some(e)) = (enumerate_start_key, enumerate_end_key) {
            info!("start-key: {}", serde_json::to_string(&s.key()?).unwrap_or_default());
            info!("end-key: {}", serde_json::to_string(&e.key()?).unwrap_or_default());
        }

        let checksum = hasher.finish();
        let elapsed = start_time.elapsed();

        Ok((checksum, elapsed))
    }
}

impl<DB: Database> TableViewer<()> for ChecksumViewer<'_, DB> {
    type Error = eyre::Report;

    fn view<T: Table>(&self) -> Result<(), Self::Error> {
        let (checksum, elapsed) = self.get_checksum::<T>()?;
        info!("Checksum for table `{}`: {:#x} (elapsed: {:?})", T::NAME, checksum, elapsed);

        Ok(())
    }
}
