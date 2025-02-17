//  Copyright 2021 Datafuse Labs.
//
//  Licensed under the Apache License, Version 2.0 (the "License");
//  you may not use this file except in compliance with the License.
//  You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.

use std::any::Any;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::sync::Arc;

use common_datablocks::DataBlock;
use common_datavalues::ColumnRef;
use common_datavalues::DataType;
use common_datavalues::Series;
use common_datavalues::StructColumn;
use common_datavalues::TypeID;
use common_exception::ErrorCode;
use common_exception::Result;
use common_legacy_planners::Extras;
use common_legacy_planners::Partitions;
use common_legacy_planners::Projection;
use common_legacy_planners::ReadDataSourcePlan;
use common_legacy_planners::Statistics;
use common_meta_app::schema::TableInfo;
use parking_lot::Mutex;
use parking_lot::RwLock;

use crate::pipelines::processors::port::InputPort;
use crate::pipelines::processors::port::OutputPort;
use crate::pipelines::processors::processor::ProcessorPtr;
use crate::pipelines::processors::ContextSink;
use crate::pipelines::processors::SyncSource;
use crate::pipelines::processors::SyncSourcer;
use crate::pipelines::Pipeline;
use crate::pipelines::SinkPipeBuilder;
use crate::pipelines::SourcePipeBuilder;
use crate::sessions::TableContext;
use crate::storages::memory::memory_part::MemoryPartInfo;
use crate::storages::StorageContext;
use crate::storages::StorageDescription;
use crate::storages::Table;

pub struct MemoryTable {
    table_info: TableInfo,
    blocks: Arc<RwLock<Vec<DataBlock>>>,
}

impl MemoryTable {
    pub fn try_create(ctx: StorageContext, table_info: TableInfo) -> Result<Box<dyn Table>> {
        let table_id = &table_info.ident.table_id;
        let blocks = {
            let mut in_mem_data = ctx.in_memory_data.write();
            let x = in_mem_data.get(table_id);
            match x {
                None => {
                    let blocks = Arc::new(RwLock::new(vec![]));
                    in_mem_data.insert(*table_id, blocks.clone());
                    blocks
                }
                Some(blocks) => blocks.clone(),
            }
        };

        let table = Self { table_info, blocks };
        Ok(Box::new(table))
    }

    pub fn description() -> StorageDescription {
        StorageDescription {
            engine_name: "MEMORY".to_string(),
            comment: "MEMORY Storage Engine".to_string(),
            ..Default::default()
        }
    }

    fn get_read_data_blocks(&self) -> Arc<Mutex<VecDeque<DataBlock>>> {
        let data_blocks = self.blocks.read();
        let mut read_data_blocks = VecDeque::with_capacity(data_blocks.len());

        for data_block in data_blocks.iter() {
            read_data_blocks.push_back(data_block.clone());
        }

        Arc::new(Mutex::new(read_data_blocks))
    }

    pub fn generate_memory_parts(start: usize, workers: usize, total: usize) -> Partitions {
        let part_size = total / workers;
        let part_remain = total % workers;

        let mut partitions = Vec::with_capacity(workers as usize);
        if part_size == 0 {
            partitions.push(MemoryPartInfo::create(start, total, total));
        } else {
            for part in 0..workers {
                let mut part_begin = part * part_size;
                if part == 0 && start > 0 {
                    part_begin = start;
                }
                let mut part_end = (part + 1) * part_size;
                if part == (workers - 1) && part_remain > 0 {
                    part_end += part_remain;
                }

                partitions.push(MemoryPartInfo::create(part_begin, part_end, total));
            }
        }

        partitions
    }
}

#[async_trait::async_trait]
impl Table for MemoryTable {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn get_table_info(&self) -> &TableInfo {
        &self.table_info
    }

    fn benefit_column_prune(&self) -> bool {
        true
    }

    async fn read_partitions(
        &self,
        ctx: Arc<dyn TableContext>,
        push_downs: Option<Extras>,
    ) -> Result<(Statistics, Partitions)> {
        let blocks = self.blocks.read();

        let statistics = match push_downs {
            Some(push_downs) => {
                let projection_filter: Box<dyn Fn(usize) -> bool> = match push_downs.projection {
                    Some(prj) => {
                        let col_ids = match prj {
                            Projection::Columns(indices) => indices,
                            Projection::InnerColumns(path_indices) => {
                                path_indices.values().map(|i| i[0]).collect()
                            }
                        };
                        let proj_cols = HashSet::<usize>::from_iter(col_ids);
                        Box::new(move |column_id: usize| proj_cols.contains(&column_id))
                    }
                    None => Box::new(|_: usize| true),
                };

                blocks
                    .iter()
                    .fold(Statistics::default(), |mut stats, block| {
                        stats.read_rows += block.num_rows() as usize;
                        stats.read_bytes += (0..block.num_columns())
                            .into_iter()
                            .collect::<Vec<usize>>()
                            .iter()
                            .filter(|cid| projection_filter(**cid))
                            .map(|cid| block.columns()[*cid].memory_size() as u64)
                            .sum::<u64>() as usize;

                        stats
                    })
            }
            None => {
                let rows = blocks.iter().map(|block| block.num_rows()).sum();
                let bytes = blocks.iter().map(|block| block.memory_size()).sum();

                Statistics::new_exact(rows, bytes, blocks.len(), blocks.len())
            }
        };

        let parts = Self::generate_memory_parts(
            0,
            ctx.get_settings().get_max_threads()? as usize,
            blocks.len(),
        );
        Ok((statistics, parts))
    }

    fn read_data(
        &self,
        ctx: Arc<dyn TableContext>,
        plan: &ReadDataSourcePlan,
        pipeline: &mut Pipeline,
    ) -> Result<()> {
        let settings = ctx.get_settings();
        let mut builder = SourcePipeBuilder::create();
        let read_data_blocks = self.get_read_data_blocks();

        for _index in 0..settings.get_max_threads()? {
            let output = OutputPort::create();
            builder.add_source(
                output.clone(),
                MemoryTableSource::create(
                    ctx.clone(),
                    output,
                    read_data_blocks.clone(),
                    plan.push_downs.clone(),
                )?,
            );
        }

        pipeline.add_pipe(builder.finalize());
        Ok(())
    }

    fn append_data(
        &self,
        ctx: Arc<dyn TableContext>,
        pipeline: &mut Pipeline,
        _: bool,
    ) -> Result<()> {
        let mut sink_pipeline_builder = SinkPipeBuilder::create();
        for _ in 0..pipeline.output_len() {
            let input_port = InputPort::create();
            sink_pipeline_builder.add_sink(
                input_port.clone(),
                ContextSink::create(input_port, ctx.clone()),
            );
        }
        pipeline.add_pipe(sink_pipeline_builder.finalize());
        Ok(())
    }

    async fn commit_insertion(
        &self,
        ctx: Arc<dyn TableContext>,
        operations: Vec<DataBlock>,
        overwrite: bool,
    ) -> Result<()> {
        let written_bytes: usize = operations.iter().map(|b| b.memory_size()).sum();

        ctx.get_dal_context()
            .get_metrics()
            .inc_write_bytes(written_bytes);

        if overwrite {
            let mut blocks = self.blocks.write();
            blocks.clear();
        }
        let mut blocks = self.blocks.write();
        for block in operations {
            blocks.push(block);
        }
        Ok(())
    }

    async fn truncate(&self, _ctx: Arc<dyn TableContext>, _: bool) -> Result<()> {
        let mut blocks = self.blocks.write();
        blocks.clear();
        Ok(())
    }
}

struct MemoryTableSource {
    extras: Option<Extras>,
    data_blocks: Arc<Mutex<VecDeque<DataBlock>>>,
}

impl MemoryTableSource {
    pub fn create(
        ctx: Arc<dyn TableContext>,
        output: Arc<OutputPort>,
        data_blocks: Arc<Mutex<VecDeque<DataBlock>>>,
        extras: Option<Extras>,
    ) -> Result<ProcessorPtr> {
        SyncSourcer::create(ctx, output, MemoryTableSource {
            extras,
            data_blocks,
        })
    }

    fn projection(&self, data_block: DataBlock) -> Result<Option<DataBlock>> {
        if let Some(extras) = &self.extras {
            if let Some(projection) = &extras.projection {
                let raw_columns = data_block.columns();
                let pruned_data_block = match projection {
                    Projection::Columns(indices) => {
                        let pruned_schema = data_block.schema().project(indices);
                        let columns = indices
                            .iter()
                            .map(|idx| raw_columns[*idx].clone())
                            .collect();
                        DataBlock::create(Arc::new(pruned_schema), columns)
                    }
                    Projection::InnerColumns(path_indices) => {
                        let pruned_schema = data_block.schema().inner_project(path_indices);
                        let mut columns = Vec::with_capacity(path_indices.len());
                        let paths: Vec<&Vec<usize>> = path_indices.values().collect();
                        for path in paths {
                            let column = Self::traverse_paths(raw_columns, path)?;
                            columns.push(column);
                        }
                        DataBlock::create(Arc::new(pruned_schema), columns)
                    }
                };
                return Ok(Some(pruned_data_block));
            }
        }

        Ok(Some(data_block))
    }

    fn traverse_paths(columns: &[ColumnRef], path: &[usize]) -> Result<ColumnRef> {
        if path.is_empty() {
            return Err(ErrorCode::BadArguments("path should not be empty"));
        }
        let column = &columns[path[0]];
        if path.len() == 1 {
            return Ok(column.clone());
        }
        if column.data_type().data_type_id() == TypeID::Struct {
            let struct_column: &StructColumn = Series::check_get(column)?;
            let inner_columns = struct_column.values();
            return Self::traverse_paths(inner_columns, &path[1..]);
        }
        Err(ErrorCode::BadArguments(format!(
            "Unable to get column by paths: {:?}",
            path
        )))
    }
}

impl SyncSource for MemoryTableSource {
    const NAME: &'static str = "MemoryTable";

    fn generate(&mut self) -> Result<Option<DataBlock>> {
        let mut blocks_guard = self.data_blocks.lock();
        match blocks_guard.pop_front() {
            None => Ok(None),
            Some(data_block) => self.projection(data_block),
        }
    }
}
