use arrow::array::Array;
use emacs::{Env, FromLisp, IntoLisp, Value, defun};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Runtime::new().expect("failed to create tokio runtime"))
}

pub mod parca {
    pub mod metastore {
        pub mod v1alpha1 {
            tonic::include_proto!("parca.metastore.v1alpha1");
        }
    }
    pub mod profilestore {
        pub mod v1alpha1 {
            tonic::include_proto!("parca.profilestore.v1alpha1");
        }
    }
    pub mod query {
        pub mod v1alpha1 {
            tonic::include_proto!("parca.query.v1alpha1");
        }
    }
}

mod auth;

struct ElispTime(SystemTime);

impl<'e> FromLisp<'e> for ElispTime {
    fn from_lisp(value: emacs::Value<'e>) -> emacs::Result<Self> {
        let since_epoch: f64 = value.into_rust()?;
        Ok(Self(UNIX_EPOCH + Duration::from_secs_f64(since_epoch)))
    }
}

#[derive(Default)]
struct SourceRowResult {
    cumulative: u64,
    flat: u64,
    lineno: u64,
}

#[derive(Default)]
struct SourceFileResult {
    rows: Vec<SourceRowResult>,
}

impl<'e> IntoLisp<'e> for SourceFileResult {
    fn into_lisp(self, env: &'e Env) -> emacs::Result<Value<'e>> {
        let nil = env.intern("nil")?;
        let conses = self
            .rows
            .into_iter()
            .map(
                |SourceRowResult {
                     cumulative,
                     flat,
                     lineno,
                 }| env.cons(lineno, env.cons(cumulative, env.cons(flat, nil)?)?),
            )
            .collect::<emacs::Result<Vec<_>>>()?;
        env.call("list", &conses)
    }
}
#[defun]
// TODO - non-blocking version
fn source_query<'e>(
    env: &'e Env,
    token: String,
    filename: String,
    build_id: String,
    project_id: String,
    query: String,
    start: ElispTime,
    end: ElispTime,
) -> emacs::Result<Value<'e>> {
    use parca::query::v1alpha1::{
        MergeProfile, QueryRequest, SourceReference,
        query_request::{Mode, Options as QROptions, ReportType},
        query_response::Report,
        query_service_client::QueryServiceClient,
    };

    let rt = runtime();

    let record = rt.block_on(async {
        let tls = tonic::transport::ClientTlsConfig::new().with_enabled_roots();

        let channel = tonic::transport::Channel::from_static("https://grpc.polarsignals.com")
            .tls_config(tls)
            .map_err(|e| anyhow::anyhow!("failed to configure TLS: {e}"))?
            .connect()
            .await
            .map_err(|e| anyhow::anyhow!("failed to connect: {e:?}"))?;

        let bearer = format!("Bearer {token}");
        let bearer_header_value: tonic::metadata::MetadataValue<_> = bearer
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid token: {e}"))?;

        let project_id_header_value: tonic::metadata::MetadataValue<_> = project_id
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid project id: {e}"))?;

        let mut client =
            QueryServiceClient::with_interceptor(channel, move |mut req: tonic::Request<()>| {
                req.metadata_mut()
                    .insert("authorization", bearer_header_value.clone());
                req.metadata_mut()
                    .insert("projectid", project_id_header_value.clone());
                Ok(req)
            });

        let request = QueryRequest {
            mode: Mode::Merge as i32,
            report_type: ReportType::Source as i32,
            source_reference: Some(SourceReference {
                build_id,
                filename,
                source_only: false,
            }),
            options: Some(QROptions::Merge(MergeProfile {
                query,
                start: Some(start.0.into()),
                end: Some(end.0.into()),
            })),
            ..Default::default()
        };

        let response = client
            .query(request)
            .await
            .map_err(|e| anyhow::anyhow!("query failed: {e}"))?;

        let query_response = response.into_inner();
        let source = match query_response.report {
            Some(Report::Source(source)) => source,
            other => {
                return Err(anyhow::anyhow!("unexpected report type: {other:?}").into());
            }
        };

        Ok::<Vec<u8>, anyhow::Error>(source.record)
    })?;

    let reader = arrow::ipc::reader::StreamReader::try_new(std::io::Cursor::new(&record), None)
        .map_err(|e| anyhow::anyhow!("failed to read arrow record: {e}"))?;

    let mut result = HashMap::<String, SourceFileResult>::new();
    for batch in reader {
        let batch = batch.map_err(|e| anyhow::anyhow!("failed to read arrow batch: {e}"))?;

        let filenames = batch
            .column_by_name("filename")
            .ok_or_else(|| anyhow::anyhow!("missing filename column"))?;
        let filenames = filenames
            .as_any()
            .downcast_ref::<arrow::array::DictionaryArray<arrow::datatypes::Int32Type>>()
            .ok_or_else(|| anyhow::anyhow!("filename column is not a dictionary"))?;
        let filename_values = filenames
            .values()
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .ok_or_else(|| anyhow::anyhow!("filename dictionary values are not strings"))?;

        let line_numbers = batch
            .column_by_name("line_number")
            .ok_or_else(|| anyhow::anyhow!("missing line_number column"))?;
        let line_numbers = line_numbers
            .as_any()
            .downcast_ref::<arrow::array::Int64Array>()
            .ok_or_else(|| anyhow::anyhow!("line_number column is not Int64"))?;

        let cumulative = batch
            .column_by_name("cumulative")
            .ok_or_else(|| anyhow::anyhow!("missing cumulative column"))?;
        let cumulative = cumulative
            .as_any()
            .downcast_ref::<arrow::array::Int64Array>()
            .ok_or_else(|| anyhow::anyhow!("cumulative column is not Int64"))?;

        let flat = batch
            .column_by_name("flat")
            .ok_or_else(|| anyhow::anyhow!("missing flat column"))?;
        let flat = flat
            .as_any()
            .downcast_ref::<arrow::array::Int64Array>()
            .ok_or_else(|| anyhow::anyhow!("flat column is not Int64"))?;

        for i in 0..batch.num_rows() {
            let fname = if filenames.is_valid(i) {
                let key = filenames.keys().value(i) as usize;
                filename_values.value(key)
            } else {
                // TODO - log error? When can filename be invalid?
                ""
            };

            result
                .entry(fname.to_string())
                .or_default()
                .rows
                .push(SourceRowResult {
                    cumulative: cumulative.value(i).try_into()?, // TODO - log error?
                    flat: flat.value(i).try_into()?,             // TODO - log error?
                    lineno: line_numbers.value(i).try_into()?,   // TODO - log error?
                });
        }
    }

    let result = result
        .into_iter()
        .map(|(k, v)| -> emacs::Result<Value<'e>> { env.cons(k, v.into_lisp(env)?) })
        .collect::<emacs::Result<Vec<_>>>()?;

    env.call("list", &result)
}

emacs::plugin_is_GPL_compatible!();
#[emacs::module(name = "polarsignals-module")]
fn init(_: &Env) -> emacs::Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install default CryptoProvider");
    Ok(())
}
