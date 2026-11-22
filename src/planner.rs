use crate::{
    backend::{
        Backend, ExtractTextBackend, ImageResampler, PageRasterizer, Qpdf, RenderPageBackend,
    },
    error::{Error, Result},
    parse_pages,
};
use serde_json::Value;
use std::{path::PathBuf, sync::Arc};

/// Keep structural inspection mandatory even when a different engine runs the job.
pub struct BackendRegistry {
    structural: Arc<dyn Backend>,
    text: Providers<dyn ExtractTextBackend>,
    render: Providers<dyn RenderPageBackend>,
    resampler: Option<Arc<dyn ImageResampler>>,
    /// One rasterizer per registry; the pageRasterization path uses it
    /// when image resampling cannot reach a byte target.
    rasterizer: Option<Arc<dyn PageRasterizer>>,
}

type Providers<T> = Vec<(&'static str, Arc<T>)>;

impl BackendRegistry {
    pub fn with_qpdf(executable: PathBuf) -> Self {
        Self::with_qpdf_exact(executable, false)
    }

    pub fn with_qpdf_exact(executable: PathBuf, exact: bool) -> Self {
        Self::with_structural(Arc::new(Qpdf {
            executable,
            exact,
            as_limit: 768 * 1024 * 1024,
            cpu_scale: 1,
        }))
    }

    pub(crate) fn with_structural(structural: Arc<dyn Backend>) -> Self {
        Self {
            structural,
            text: Vec::new(),
            render: Vec::new(),
            resampler: None,
            rasterizer: None,
        }
    }

    /// One resampler per registry; the compression ladder uses it when present.
    pub fn register_resampler(&mut self, provider: Arc<dyn ImageResampler>) {
        self.resampler = Some(provider);
    }

    /// One rasterizer per registry; pageRasterization uses it when present.
    pub fn register_rasterizer(&mut self, provider: Arc<dyn PageRasterizer>) {
        self.rasterizer = Some(provider);
    }

    pub fn register_text(
        &mut self,
        engine: &'static str,
        provider: Arc<dyn ExtractTextBackend>,
    ) -> Result<()> {
        register(&mut self.text, engine, provider)
    }

    pub fn register_renderer(
        &mut self,
        engine: &'static str,
        provider: Arc<dyn RenderPageBackend>,
    ) -> Result<()> {
        register(&mut self.render, engine, provider)
    }

    pub(crate) fn engines(&self, id: &str) -> Vec<&'static str> {
        match capability(id) {
            Ok(Capability::Text) => self.text.iter().map(|(id, _)| *id).collect(),
            Ok(Capability::Render) => self.render.iter().map(|(id, _)| *id).collect(),
            Ok(_) => vec!["qpdf"],
            Err(_) => Vec::new(),
        }
    }

    pub(crate) fn plan(&self, id: &str, request: &Value) -> Result<Plan<'_>> {
        let engine = request["engine"].as_str();
        let job = match capability(id)? {
            Capability::Inspect => Job::Inspect,
            Capability::Transform => Job::Transform,
            Capability::Compress => Job::Compress,
            Capability::Text => {
                let pages = parse_pages(&request["pages"])?;
                let (engine, backend) = select(&self.text, engine)?;
                Job::ExtractText {
                    engine,
                    backend,
                    pages,
                }
            }
            Capability::Render => {
                let page = crate::request_integer(&request["page"])?;
                let dpi = request
                    .get("dpi")
                    .map(crate::request_integer)
                    .transpose()?
                    .unwrap_or(144) as u32;
                let (engine, backend) = select(&self.render, engine)?;
                Job::RenderPage {
                    engine,
                    backend,
                    page,
                    dpi,
                }
            }
        };
        Ok(Plan {
            structural: self.structural.as_ref(),
            resampler: self.resampler.as_deref(),
            rasterizer: self.rasterizer.as_deref(),
            job,
        })
    }
}

fn register<T: ?Sized>(
    providers: &mut Providers<T>,
    engine: &'static str,
    provider: Arc<T>,
) -> Result<()> {
    let mut chars = engine.bytes();
    if engine.len() > 32
        || !chars.next().is_some_and(|c| c.is_ascii_lowercase())
        || !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        || providers.iter().any(|(id, _)| *id == engine)
    {
        return Err(Error("INVALID_REQUEST", None));
    }
    providers.push((engine, provider));
    Ok(())
}

fn select<'a, T: ?Sized>(
    providers: &'a Providers<T>,
    engine: Option<&str>,
) -> Result<(&'static str, &'a T)> {
    providers
        .iter()
        .find(|(id, _)| engine.is_none_or(|name| name == *id))
        .map(|(name, provider)| (*name, provider.as_ref()))
        .ok_or(Error("BACKEND_UNAVAILABLE", None))
}

enum Capability {
    Inspect,
    Transform,
    Compress,
    Text,
    Render,
}

fn capability(id: &str) -> Result<Capability> {
    match id {
        "inspectPdf" => Ok(Capability::Inspect),
        "mergePdf" | "extractPdf" | "splitPdf" | "removePages" => Ok(Capability::Transform),
        "compressPdf" => Ok(Capability::Compress),
        "extractText" => Ok(Capability::Text),
        "renderPage" => Ok(Capability::Render),
        _ => Err(Error("UNSUPPORTED_OPERATION", None)),
    }
}

pub(crate) struct Plan<'a> {
    pub(crate) structural: &'a dyn Backend,
    pub(crate) resampler: Option<&'a dyn ImageResampler>,
    pub(crate) rasterizer: Option<&'a dyn PageRasterizer>,
    pub(crate) job: Job<'a>,
}

pub(crate) enum Job<'a> {
    Inspect,
    Transform,
    Compress,
    ExtractText {
        engine: &'static str,
        backend: &'a dyn ExtractTextBackend,
        pages: Vec<u64>,
    },
    RenderPage {
        engine: &'static str,
        backend: &'a dyn RenderPageBackend,
        page: u64,
        dpi: u32,
    },
}

impl Job<'_> {
    pub(crate) fn writes_pdf(&self) -> bool {
        matches!(self, Self::Transform | Self::Compress)
    }

    pub(crate) fn writes_output(&self) -> bool {
        self.writes_pdf() || matches!(self, Self::RenderPage { .. })
    }
}
