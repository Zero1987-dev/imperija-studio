// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! One ONNX model, loaded and ready to run.
//!
//! A thin coat over ONNX Runtime: a session with the platform's
//! accelerator registered ahead of the CPU - CoreML on Apple, DirectML on
//! Windows, NNAPI on Android - and one call that takes named `f32`
//! tensors and gives named `f32` tensors back. Every model this crate
//! runs is a picture in and a picture out, so that is all the surface it
//! needs; what each model's tensors mean lives beside the model.

use ort::session::Session;
use ort::value::Tensor;

/// Makes sure the runtime that answers is the one shipped beside the
/// program, and not whatever the linker happened to leave in the binary.
///
/// sherpa-onnx carries its own ONNX Runtime and links it statically. Those
/// symbols satisfy this crate's at link time, so the shared copy the build
/// fetched is never bound and every session here quietly runs on sherpa's -
/// an older build, made against an older standard library. On Fedora 44
/// that one aborts before it has opened a single model:
///
/// ```text
/// free(): invalid pointer
/// onnxruntime::DeviceDiscovery::DiscoverDevicesForPlatform()
/// ```
///
/// Read out of a core dump, after two guesses at it that were wrong.
///
/// Loading by path at run time binds nothing at link time, so there is
/// nothing for the other copy to capture. Once, before the first session,
/// and quiet about failing: a path that is not there leaves the loader to
/// find the library the ordinary way, which is what a developer's build
/// wants anyway.
#[cfg(target_os = "linux")]
pub fn point_at_the_bundled_runtime() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // An environment that already says where it is has been set up
        // deliberately; do not argue with it.
        if std::env::var_os("ORT_DYLIB_PATH").is_some() {
            return;
        }
        let beside = std::env::current_exe().ok().and_then(|exe| {
            let dir = exe.parent()?;
            // The AppImage lays it in `lib/` beside the binary; a plain
            // build may leave it alongside.
            [dir.join("lib").join(LIBRARY), dir.join(LIBRARY)]
                .into_iter()
                .find(|path| path.is_file())
        });
        let Some(path) = beside else {
            return;
        };
        if let Err(error) = ort::init_from(path.to_string_lossy().as_ref()).commit() {
            // Straight to stderr: this crate carries no logger of its own,
            // and the journal keeps stderr for exactly this sort of thing.
            eprintln!("onnx runtime at {}: {error}", path.display());
        }
    });
}

/// The file name the runtime goes by on this platform.
#[cfg(target_os = "linux")]
const LIBRARY: &str = "libonnxruntime.so";

/// Everywhere else the runtime is linked the ordinary way and there is
/// only one of it.
#[cfg(not(target_os = "linux"))]
pub fn point_at_the_bundled_runtime() {}

/// A tensor by name: its dimensions and its values, row-major.
pub struct Input<'a> {
    /// The graph's input name.
    pub name: &'a str,
    /// The tensor's shape.
    pub dims: Vec<usize>,
    /// The values, `dims` product of them.
    pub data: Data,
}

/// A tensor's values. Pictures and probabilities are `f32`; the brush
/// model's point labels are `i64`.
pub enum Data {
    /// Floating point values.
    F32(Vec<f32>),
    /// Integer values.
    I64(Vec<i64>),
}

impl From<Vec<f32>> for Data {
    fn from(values: Vec<f32>) -> Data {
        Data::F32(values)
    }
}

impl From<Vec<i64>> for Data {
    fn from(values: Vec<i64>) -> Data {
        Data::I64(values)
    }
}

/// An `f32` tensor a model answered with.
#[derive(Clone, Debug)]
pub struct Output {
    /// The tensor's shape.
    pub dims: Vec<usize>,
    /// The values, row-major.
    pub data: Vec<f32>,
}

/// A loaded model.
pub struct Model {
    session: Session,
}

impl Model {
    /// Loads a model from its bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Model, String> {
        point_at_the_bundled_runtime();
        let mut builder = Session::builder().map_err(|error| format!("onnx runtime: {error}"))?;
        builder = builder
            .with_execution_providers(accelerators())
            .map_err(|error| format!("onnx runtime: {error}"))?;
        // Every core on one frame at a time: the models are convolutional
        // and a frame fills a machine on its own.
        let threads = std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(1)
            .clamp(1, 8);
        builder = builder
            .with_intra_threads(threads)
            .map_err(|error| format!("onnx runtime: {error}"))?;
        let session = builder
            .commit_from_memory(bytes)
            .map_err(|error| format!("cutout model: {error}"))?;
        Ok(Model { session })
    }

    /// Loads a model from a file, after making sure the right runtime is
    /// the one that answers.
    pub fn from_file(path: &std::path::Path) -> Result<Model, String> {
        let bytes = std::fs::read(path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        Model::from_bytes(&bytes)
    }

    /// Runs the model on `inputs` and returns the named outputs, in the
    /// order asked for.
    pub fn run(&mut self, inputs: Vec<Input<'_>>, outputs: &[&str]) -> Result<Vec<Output>, String> {
        let mut fed: Vec<(
            std::borrow::Cow<'static, str>,
            ort::session::SessionInputValue<'_>,
        )> = Vec::with_capacity(inputs.len());
        for input in inputs {
            let value: ort::session::SessionInputValue<'_> = match input.data {
                Data::F32(data) => Tensor::from_array((input.dims, data))
                    .map_err(|error| format!("cutout input {}: {error}", input.name))?
                    .into(),
                Data::I64(data) => Tensor::from_array((input.dims, data))
                    .map_err(|error| format!("cutout input {}: {error}", input.name))?
                    .into(),
            };
            fed.push((input.name.to_owned().into(), value));
        }
        let answered = self
            .session
            .run(fed)
            .map_err(|error| format!("cutout: {error}"))?;
        outputs
            .iter()
            .map(|name| {
                let value = answered
                    .get(*name)
                    .ok_or_else(|| format!("cutout: the model has no output {name:?}"))?;
                let (shape, data) = value
                    .try_extract_tensor::<f32>()
                    .map_err(|error| format!("cutout output {name}: {error}"))?;
                Ok(Output {
                    dims: shape.iter().map(|&d| d.max(0) as usize).collect(),
                    data: data.to_vec(),
                })
            })
            .collect()
    }
}

/// The platform's accelerators, ahead of the CPU. A provider the machine
/// cannot supply is skipped, not an error, so the same build runs on a
/// box without one.
fn accelerators() -> Vec<ort::ep::ExecutionProviderDispatch> {
    platform_accelerator()
        .into_iter()
        .chain(std::iter::once(ort::ep::CPU::default().build()))
        .collect()
}

#[cfg(target_vendor = "apple")]
fn platform_accelerator() -> Option<ort::ep::ExecutionProviderDispatch> {
    Some(ort::ep::CoreML::default().build())
}

#[cfg(target_os = "windows")]
fn platform_accelerator() -> Option<ort::ep::ExecutionProviderDispatch> {
    Some(ort::ep::DirectML::default().build())
}

#[cfg(target_os = "android")]
fn platform_accelerator() -> Option<ort::ep::ExecutionProviderDispatch> {
    Some(ort::ep::NNAPI::default().build())
}

#[cfg(not(any(target_vendor = "apple", target_os = "windows", target_os = "android")))]
fn platform_accelerator() -> Option<ort::ep::ExecutionProviderDispatch> {
    None
}
