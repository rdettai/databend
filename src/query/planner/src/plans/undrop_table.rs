// Copyright 2022 Datafuse Labs.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
use std::sync::Arc;

use common_datavalues::DataSchema;
use common_datavalues::DataSchemaRef;
use common_meta_app::schema::TableNameIdent;
use common_meta_app::schema::UndropTableReq;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UndropTablePlan {
    pub tenant: String,
    pub catalog: String,
    pub database: String,
    pub table: String,
}

impl UndropTablePlan {
    pub fn schema(&self) -> DataSchemaRef {
        Arc::new(DataSchema::empty())
    }
}

/// The table name
impl From<UndropTablePlan> for UndropTableReq {
    fn from(p: UndropTablePlan) -> Self {
        UndropTableReq {
            name_ident: TableNameIdent {
                tenant: p.tenant,
                db_name: p.database,
                table_name: p.table,
            },
        }
    }
}
