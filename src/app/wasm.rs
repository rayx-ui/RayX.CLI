use crate::app::args::{ensure_empty, take_bool_flag, take_flag_value};
use crate::app::assets;
use crate::app::playwright;
use crate::app::process::run as run_process;
use crate::app::{AppDescriptor, AppFeatureSelection, BuildProfile};
use anyhow::{Context, Result, anyhow, bail};
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const WASM_RUSTFLAGS: &str = "-C target-feature=+atomics,+bulk-memory,+mutable-globals -C link-arg=--shared-memory -C link-arg=--import-memory -C link-arg=--max-memory=2147483648 -C link-arg=--export=__heap_base -C link-arg=--export=__wasm_init_tls -C link-arg=--export=__tls_size -C link-arg=--export=__tls_align -C link-arg=--export=__tls_base -Awarnings";
const WASM_CFLAGS: &str = "-matomics -mbulk-memory -mmutable-globals";

fn html_island_diagnostics_bootstrap() -> &'static str {
    r#"
    const detectHtmlIslandSupport = () => {
      const probeCanvas = document.createElement('canvas');
      const canvasContext = probeCanvas.getContext?.('2d');
      const hasMethod = (owner, name) => owner != null && typeof owner[name] === 'function';
      const hasProperty = (owner, name) => owner != null && name in owner;
      const canvasLayoutSubtree = hasProperty(probeCanvas, 'layoutSubtree')
        ? 'layoutSubtree'
        : hasProperty(probeCanvas, 'layoutsubtree')
          ? 'layoutsubtree'
          : null;
      const probes = {
        canvasLayoutSubtree,
        canvasPaintEvent: hasProperty(probeCanvas, 'onpaint') ? 'paint' : null,
        canvasRequestPaint: hasMethod(probeCanvas, 'requestPaint') ? 'requestPaint' : null,
        globalCaptureElementImage: hasMethod(globalThis, 'captureElementImage')
          ? 'captureElementImage'
          : null,
        elementCaptureElementImage: hasMethod(globalThis.Element?.prototype, 'captureElementImage')
          ? 'Element.prototype.captureElementImage'
          : null,
        canvasDrawElementImage: hasMethod(canvasContext, 'drawElementImage')
          ? 'CanvasRenderingContext2D.drawElementImage'
          : null,
        gpuQueueCopyElementImageToTexture: hasMethod(
          globalThis.GPUQueue?.prototype,
          'copyElementImageToTexture',
        )
          ? 'GPUQueue.copyElementImageToTexture'
          : null,
        gpuQueueCopyExternalImageToTexture: hasMethod(
          globalThis.GPUQueue?.prototype,
          'copyExternalImageToTexture',
        )
          ? 'GPUQueue.copyExternalImageToTexture'
          : null,
      };
      const support = {
        layoutSubtree: probes.canvasLayoutSubtree !== null,
        paintEvent: probes.canvasPaintEvent !== null,
        requestPaint: probes.canvasRequestPaint !== null,
        captureElementImage:
          probes.globalCaptureElementImage !== null || probes.elementCaptureElementImage !== null,
        drawElementImage: probes.canvasDrawElementImage !== null,
        copyElementImageToTexture: probes.gpuQueueCopyElementImageToTexture !== null,
        copyExternalImageToTexture: probes.gpuQueueCopyExternalImageToTexture !== null,
      };
      return { probes, support };
    };
    const htmlIslandSupport = detectHtmlIslandSupport();
    const htmlIslandSearch = new URLSearchParams(globalThis.location?.search ?? '');
    const htmlIslandEnabledValue = htmlIslandSearch.get('rayx-html-island');
    globalThis.__rayxHtmlIslandEnabled =
      htmlIslandEnabledValue === '1' || htmlIslandEnabledValue === 'true';
    globalThis.__rayxHtmlIslandDiagnostics = {
      version: 1,
      available: false,
      probes: htmlIslandSupport.probes,
      support: htmlIslandSupport.support,
      canvas: {
        layoutSubtreeEnabled: false,
        childCount: 0,
      },
      islands: {
        count: 0,
        dirtyCount: 0,
        uploadCount: 0,
        textureReallocationCount: 0,
      },
      renderer: {
        path: 'unsupported',
        lastError: null,
      },
    };
    globalThis.__rayxHtmlIslandDiagnostics.available =
      htmlIslandSupport.support.layoutSubtree
      && htmlIslandSupport.support.paintEvent
      && htmlIslandSupport.support.requestPaint
      && htmlIslandSupport.support.captureElementImage
      && htmlIslandSupport.support.copyElementImageToTexture;
"#
}

pub fn build(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    profile: BuildProfile,
    args: Vec<String>,
) -> Result<PathBuf> {
    ensure_empty(&args)?;
    build_with_profile(app, profile, features)
}

pub fn run(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    profile: BuildProfile,
    mut args: Vec<String>,
) -> Result<()> {
    let port = take_port(&mut args)?;
    ensure_empty(&args)?;
    let out_dir = build_with_profile(app, profile, features)?;
    serve(&out_dir, port, None)
}

pub fn test(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    profile: BuildProfile,
    mut args: Vec<String>,
) -> Result<()> {
    let headed = take_bool_flag(&mut args, "--headed");
    let webgpu = take_bool_flag(&mut args, "--webgpu");
    let port = take_port(&mut args)?;
    let selectors = take_playwright_tests(&mut args)?;
    ensure_empty(&args)?;
    let package = playwright::resolve(app)?;
    if package.created {
        println!(
            "Created the Playwright package {} from the RayX template: commit it.",
            crate::app::fs_util::command_path(&package.dir).display()
        );
    }
    let specs = playwright::select_specs(app, &package, &selectors)?;
    let projects = playwright::select_projects(app)?;
    let test_features = wasm_test_features(app.diagnostic_harness, features);
    let out_dir = prepare_before_build(
        || playwright::prepare(app, &package),
        || build_with_profile(app, profile, &test_features),
    )?;
    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();
    let server_dir = out_dir.clone();
    // Bind here, not in the server thread: a leftover server on the port must fail this run
    // instead of being mistaken for the build that was just made.
    let listener = bind_listener(port)?;
    let server = thread::spawn(move || serve_on(listener, &server_dir, port, Some(shutdown_rx)));
    wait_for_server(port)?;

    let url = format!("http://127.0.0.1:{port}/");
    let feature_list = playwright_feature_list(&test_features);
    let mut command = playwright::test_command(
        app,
        &package,
        &specs,
        &projects,
        &playwright::TestRun {
            url: &url,
            slug: &app.slug,
            features: &feature_list,
            headed,
            webgpu,
        },
    );
    let result = run_process(&mut command);
    let _ = shutdown_tx.send(());
    match server.join() {
        Ok(server_result) => server_result?,
        Err(_) => bail!("wasm server thread panicked"),
    }
    result
}

fn wasm_test_features(
    diagnostic_harness: bool,
    features: &AppFeatureSelection,
) -> AppFeatureSelection {
    let mut selected = features.clone();
    if diagnostic_harness && !diagnostics_enabled(&selected) {
        selected.features.push("rayx_diagnostics".to_string());
    }
    selected
}

/// The app features the served build enables, for specs that need an opt-in
/// fixture feature: a comma-separated list, or `*` for `--all-features`.
fn playwright_feature_list(features: &AppFeatureSelection) -> String {
    if features.all_features {
        "*".to_string()
    } else {
        features.normalized_features().join(",")
    }
}

fn diagnostics_enabled(features: &AppFeatureSelection) -> bool {
    features.all_features
        || features
            .normalized_features()
            .iter()
            .any(|feature| feature == "rayx_diagnostics")
}

fn prepare_before_build<P, B>(prepare: P, build: B) -> Result<PathBuf>
where
    P: FnOnce() -> Result<()>,
    B: FnOnce() -> Result<PathBuf>,
{
    prepare()?;
    build()
}

/// The `wasm-bindgen` command-line tool must be the version the project locks: the generated
/// bindings and the CLI have to agree.
fn ensure_wasm_bindgen_cli(app: &AppDescriptor) -> Result<()> {
    let Some(locked) = app.context.pins.wasm_bindgen.as_ref() else {
        return Ok(());
    };
    let output = Command::new("wasm-bindgen")
        .arg("--version")
        .output()
        .map_err(|_| {
            anyhow!(
                "wasm-bindgen {} is not installed: run `rayx setup --web`",
                locked.value
            )
        })?;
    let text = String::from_utf8_lossy(&output.stdout);
    check_wasm_bindgen_version(
        text.split_whitespace().nth(1).unwrap_or_default(),
        &locked.value,
    )
}

/// Errors unless the installed `wasm-bindgen` CLI version is the one the project locks.
pub fn check_wasm_bindgen_version(installed: &str, locked: &str) -> Result<()> {
    if installed != locked {
        bail!(
            "the wasm-bindgen CLI is {installed} but Cargo.lock locks {locked}: run `rayx setup --web`"
        );
    }
    Ok(())
}

/// The `cargo build` of the generated web entry crate: the pinned nightly with `build-std`, the
/// shared-memory flags, and the workspace's own target directory.
pub fn cargo_build_command(
    app: &AppDescriptor,
    web_manifest: &Path,
    profile: BuildProfile,
) -> Result<Command> {
    let mut cargo = Command::new("cargo");
    cargo
        .arg(format!("+{}", app.context.pins.web_toolchain.value))
        .arg("build")
        .arg("--manifest-path")
        .arg(web_manifest)
        .args(["--target", "wasm32-unknown-unknown", "--target-dir"])
        .arg(app.target_dir()?)
        .args(["-Z", "build-std=std,panic_abort"])
        .env("RUSTFLAGS", WASM_RUSTFLAGS)
        .env("CFLAGS_wasm32_unknown_unknown", WASM_CFLAGS)
        .env("CXXFLAGS_wasm32_unknown_unknown", WASM_CFLAGS)
        .env("CARGO_FUTURE_INCOMPAT_REPORT_FREQUENCY", "never");
    if let Some(profile_arg) = profile.cargo_arg() {
        cargo.arg(profile_arg);
    }
    Ok(cargo)
}

fn build_with_profile(
    app: &AppDescriptor,
    profile: BuildProfile,
    features: &AppFeatureSelection,
) -> Result<PathBuf> {
    let root = app.context.workspace_root().to_path_buf();
    ensure_wasm_bindgen_cli(app)?;
    let web_manifest = app.web_manifest(features)?;
    let module_base = app.web_module_base();
    let mut cargo = cargo_build_command(app, &web_manifest, profile)?;
    run_process(&mut cargo)?;

    let out_dir = app.artifact_root()?.join("wasm").join(profile.name());
    fs::create_dir_all(&out_dir)?;
    let input = root
        .join("target")
        .join("wasm32-unknown-unknown")
        .join(profile.name())
        .join(format!("{module_base}.wasm"));
    run_process(
        Command::new("wasm-bindgen").args([
            input
                .to_str()
                .ok_or_else(|| anyhow!("invalid wasm input path"))?,
            "--target",
            "web",
            "--out-dir",
            out_dir
                .to_str()
                .ok_or_else(|| anyhow!("invalid wasm output path"))?,
        ]),
    )?;
    verify_threaded_wasm(&out_dir.join(format!("{module_base}_bg.wasm")))?;
    write_wasm_host(
        &out_dir,
        &module_base,
        &app.slug,
        diagnostics_enabled(features),
    )?;
    copy_app_assets(app, features, &out_dir)?;
    assets::package_app_content(app, &out_dir)?;
    println!(
        "WASM host written to {}",
        out_dir.join("index.html").display()
    );
    Ok(out_dir)
}

fn take_port(args: &mut Vec<String>) -> Result<u16> {
    let Some(value) = take_flag_value(args, "--port")? else {
        return Ok(7878);
    };
    value
        .parse::<u16>()
        .with_context(|| format!("invalid --port value {value}"))
}

/// `--playwright-test <spec>...`: the flag value plus every following
/// non-flag argument, so several specs run in one Playwright invocation.
fn take_playwright_tests(args: &mut Vec<String>) -> Result<Vec<String>> {
    let mut specs = Vec::new();
    while let Some(index) = args.iter().position(|arg| arg == "--playwright-test") {
        let _ = args.remove(index);
        if index >= args.len() || args[index].starts_with("--") {
            bail!("--playwright-test expects a value");
        }
        while index < args.len() && !args[index].starts_with("--") {
            specs.push(args.remove(index));
        }
    }
    Ok(specs)
}

fn copy_app_assets(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    out_dir: &Path,
) -> Result<PathBuf> {
    let destination = out_dir.join("assets");
    // The fonts gpux's SVG renderer loads through the app's asset source (`fonts/...`); an app
    // that does not use gpux has none.
    let mut additional_roots = Vec::new();
    if let Some(fonts) = app
        .context
        .find_resolved_package_dir("gpux-fonts", &app.manifest())?
    {
        let gpux_assets = fonts.join("assets");
        if !gpux_assets.is_dir() {
            bail!("gpux font assets are missing at {}", gpux_assets.display());
        }
        additional_roots.push(gpux_assets);
    }
    assets::package_app_assets_with_additional_roots(
        app,
        features,
        &destination,
        &additional_roots,
    )?;
    Ok(destination)
}

fn verify_threaded_wasm(wasm_path: &Path) -> Result<()> {
    let wasm = fs::read(wasm_path)
        .with_context(|| format!("failed to read generated wasm {}", wasm_path.display()))?;
    if wasm.get(..8) != Some(b"\0asm\x01\0\0\0") {
        bail!("{} is not a WebAssembly module", wasm_path.display());
    }

    let mut offset = 8;
    let mut shared_memory = false;
    let mut thread_entry_export = false;
    while offset < wasm.len() {
        let section_id = read_wasm_byte(&wasm, &mut offset, wasm.len())?;
        let section_len = read_wasm_u32(&wasm, &mut offset, wasm.len())? as usize;
        let section_end = offset
            .checked_add(section_len)
            .filter(|end| *end <= wasm.len())
            .ok_or_else(|| anyhow!("malformed wasm section length in {}", wasm_path.display()))?;
        let mut section_offset = offset;
        match section_id {
            2 => parse_wasm_imports(&wasm, &mut section_offset, section_end, &mut shared_memory)?,
            5 => parse_wasm_memory_section(
                &wasm,
                &mut section_offset,
                section_end,
                &mut shared_memory,
            )?,
            7 => parse_wasm_exports(
                &wasm,
                &mut section_offset,
                section_end,
                &mut thread_entry_export,
            )?,
            _ => {}
        }
        offset = section_end;
    }

    let atomic_prefix_count = wasm.iter().filter(|byte| **byte == 0xfe).count();
    if !shared_memory {
        bail!(
            "{} was built without shared WebAssembly memory",
            wasm_path.display()
        );
    }
    if !thread_entry_export {
        bail!(
            "{} does not export wasm_thread_entry_point",
            wasm_path.display()
        );
    }
    if atomic_prefix_count == 0 {
        bail!(
            "{} contains no WebAssembly atomic instruction prefixes",
            wasm_path.display()
        );
    }

    println!(
        "Verified multithreaded WASM: shared memory, wasm_thread_entry_point, {atomic_prefix_count} atomic prefix byte(s)"
    );
    Ok(())
}

fn parse_wasm_imports(
    wasm: &[u8],
    offset: &mut usize,
    section_end: usize,
    shared_memory: &mut bool,
) -> Result<()> {
    let count = read_wasm_u32(wasm, offset, section_end)?;
    for _ in 0..count {
        let _module = read_wasm_name(wasm, offset, section_end)?;
        let _name = read_wasm_name(wasm, offset, section_end)?;
        match read_wasm_byte(wasm, offset, section_end)? {
            0 => {
                let _type_index = read_wasm_u32(wasm, offset, section_end)?;
            }
            1 => {
                let _element_type = read_wasm_byte(wasm, offset, section_end)?;
                let _ = read_wasm_limits(wasm, offset, section_end)?;
            }
            2 => {
                *shared_memory |= read_wasm_limits(wasm, offset, section_end)?;
            }
            3 => {
                let _value_type = read_wasm_byte(wasm, offset, section_end)?;
                let _mutable = read_wasm_byte(wasm, offset, section_end)?;
            }
            4 => {
                let _attribute = read_wasm_u32(wasm, offset, section_end)?;
                let _type_index = read_wasm_u32(wasm, offset, section_end)?;
            }
            other => bail!("unsupported wasm import descriptor kind {other}"),
        }
    }
    Ok(())
}

fn parse_wasm_memory_section(
    wasm: &[u8],
    offset: &mut usize,
    section_end: usize,
    shared_memory: &mut bool,
) -> Result<()> {
    let count = read_wasm_u32(wasm, offset, section_end)?;
    for _ in 0..count {
        *shared_memory |= read_wasm_limits(wasm, offset, section_end)?;
    }
    Ok(())
}

fn parse_wasm_exports(
    wasm: &[u8],
    offset: &mut usize,
    section_end: usize,
    thread_entry_export: &mut bool,
) -> Result<()> {
    let count = read_wasm_u32(wasm, offset, section_end)?;
    for _ in 0..count {
        let name = read_wasm_name(wasm, offset, section_end)?;
        let _kind = read_wasm_byte(wasm, offset, section_end)?;
        let _index = read_wasm_u32(wasm, offset, section_end)?;
        if name == "wasm_thread_entry_point" {
            *thread_entry_export = true;
        }
    }
    Ok(())
}

fn read_wasm_limits(wasm: &[u8], offset: &mut usize, section_end: usize) -> Result<bool> {
    let flags = read_wasm_byte(wasm, offset, section_end)?;
    if flags & !0x03 != 0 {
        bail!("unsupported wasm limits flags {flags:#04x}");
    }
    let shared = flags & 0x02 != 0;
    let has_max = flags & 0x01 != 0 || shared;
    let _initial = read_wasm_u32(wasm, offset, section_end)?;
    if has_max {
        let _maximum = read_wasm_u32(wasm, offset, section_end)?;
    }
    Ok(shared)
}

fn read_wasm_name(wasm: &[u8], offset: &mut usize, section_end: usize) -> Result<String> {
    let len = read_wasm_u32(wasm, offset, section_end)? as usize;
    let end = offset
        .checked_add(len)
        .filter(|end| *end <= section_end)
        .ok_or_else(|| anyhow!("malformed wasm name length"))?;
    let name = std::str::from_utf8(&wasm[*offset..end])
        .context("wasm name is not valid utf-8")?
        .to_string();
    *offset = end;
    Ok(name)
}

fn read_wasm_u32(wasm: &[u8], offset: &mut usize, section_end: usize) -> Result<u32> {
    let mut value = 0u32;
    let mut shift = 0u32;
    loop {
        let byte = read_wasm_byte(wasm, offset, section_end)?;
        value |= ((byte & 0x7f) as u32) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
        if shift >= 35 {
            bail!("malformed wasm u32 LEB128 value");
        }
    }
}

fn read_wasm_byte(wasm: &[u8], offset: &mut usize, section_end: usize) -> Result<u8> {
    if *offset >= section_end {
        bail!("unexpected end of wasm section");
    }
    let byte = wasm[*offset];
    *offset += 1;
    Ok(byte)
}

fn write_wasm_host(
    out_dir: &Path,
    module_base: &str,
    title: &str,
    diagnostics_enabled: bool,
) -> Result<()> {
    let html = r#"<!doctype html>
<html lang="en">
<head>
  <base href="/" />
  <meta charset="utf-8" />
  <base href="/" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <link rel="icon" href="data:," />
  <title>__TITLE__ WASM</title>
  <style>
    *, *::before, *::after { box-sizing: border-box; }
    html, body { width: 100vw; height: 100vh; margin: 0; padding: 0; overflow: hidden; background: #f8fafc; color: #111827; }
    body { font-family: system-ui, sans-serif; }
    __DIAGNOSTIC_STYLE__
  </style>
</head>
<body>
  __DIAGNOSTIC_STATUS__
  <script type="module">
    import init, { run } from '/__MODULE__.js';
__HTML_ISLAND_DIAGNOSTICS__
__DIAGNOSTIC_MARK_STATUS__
    __DIAGNOSTIC_BEFORE_RUN__
    await init();
    await run();
    __DIAGNOSTIC_AFTER_RUN__
    document.body.dataset.rayxWasm = 'started';
  </script>
</body>
</html>
"#
    .replace("__MODULE__", module_base)
    .replace("__TITLE__", title)
    .replace(
        "__DIAGNOSTIC_STYLE__",
        if diagnostics_enabled { "#webgpu-status, #wasm-thread-status { display: none; }" } else { "" },
    )
    .replace(
        "__DIAGNOSTIC_STATUS__",
        if diagnostics_enabled {
            "<div id=\"wasm-thread-status\" data-threads=\"checking\">WASM threads: checking</div>\n  <div id=\"webgpu-status\" data-webgpu=\"checking\">WebGPU: checking</div>"
        } else { "" },
    )
    .replace(
        "__HTML_ISLAND_DIAGNOSTICS__",
        if diagnostics_enabled { html_island_diagnostics_bootstrap() } else { "" },
    )
    .replace(
        "__DIAGNOSTIC_MARK_STATUS__",
        if diagnostics_enabled {
            "    const webgpuStatus = document.getElementById('webgpu-status');\n    const threadStatus = document.getElementById('wasm-thread-status');\n    const markStatus = () => {\n      const threadDiagnostics = { crossOriginIsolated: globalThis.crossOriginIsolated === true, sharedArrayBufferAvailable: typeof globalThis.SharedArrayBuffer === 'function', atomicsAvailable: typeof globalThis.Atomics === 'object', atomicsWaitAsyncAvailable: typeof globalThis.Atomics?.waitAsync === 'function' };\n      const threadsAvailable = threadDiagnostics.crossOriginIsolated && threadDiagnostics.sharedArrayBufferAvailable && threadDiagnostics.atomicsAvailable && threadDiagnostics.atomicsWaitAsyncAvailable;\n      webgpuStatus.textContent = navigator.gpu ? 'WebGPU: available' : 'WebGPU: unavailable';\n      webgpuStatus.dataset.webgpu = navigator.gpu ? 'available' : 'unavailable';\n      threadStatus.textContent = threadsAvailable ? 'WASM threads: available' : 'WASM threads: unavailable';\n      threadStatus.dataset.threads = threadsAvailable ? 'available' : 'unavailable';\n      document.body.dataset.rayxThreads = threadStatus.dataset.threads;\n      globalThis.__rayxThreadDiagnostics = threadDiagnostics;\n    };"
        } else { "" },
    )
    .replace("__DIAGNOSTIC_BEFORE_RUN__", if diagnostics_enabled { "markStatus();" } else { "" })
    .replace("__DIAGNOSTIC_AFTER_RUN__", if diagnostics_enabled { "markStatus();" } else { "" });
    fs::write(out_dir.join("index.html"), html)?;
    Ok(())
}

fn wait_for_server(port: u16) -> Result<()> {
    let start = std::time::Instant::now();
    let timeout = Duration::from_secs(10);
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(_) => return Ok(()),
            Err(error) if start.elapsed() < timeout => {
                let _ = error;
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("wasm server did not start on port {port}"));
            }
        }
    }
}

fn bind_listener(port: u16) -> Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", port)).with_context(|| {
        format!(
            "port {port} is in use (an earlier `rayx app run wasm` still running?): pass --port"
        )
    })
}

fn serve(root: &Path, port: u16, shutdown: Option<mpsc::Receiver<()>>) -> Result<()> {
    serve_on(bind_listener(port)?, root, port, shutdown)
}

fn serve_on(
    listener: TcpListener,
    root: &Path,
    port: u16,
    shutdown: Option<mpsc::Receiver<()>>,
) -> Result<()> {
    listener.set_nonblocking(true)?;
    let (connection_error_tx, connection_error_rx) = mpsc::channel();
    println!("Serving WASM at http://127.0.0.1:{port}/");
    loop {
        if shutdown.as_ref().is_some_and(|rx| rx.try_recv().is_ok()) {
            break;
        }
        if let Ok(error) = connection_error_rx.try_recv() {
            return Err(error);
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let root = root.to_path_buf();
                let connection_error_tx = connection_error_tx.clone();
                thread::spawn(move || {
                    if let Err(error) = handle_connection(stream, &root)
                        && !is_client_disconnect(&error)
                    {
                        let _ = connection_error_tx.send(error);
                    }
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn is_client_disconnect(error: &anyhow::Error) -> bool {
    error.downcast_ref::<std::io::Error>().is_some_and(|error| {
        matches!(
            error.kind(),
            std::io::ErrorKind::BrokenPipe
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::UnexpectedEof
        )
    })
}

fn handle_connection(mut stream: TcpStream, root: &Path) -> Result<()> {
    stream.set_nonblocking(false)?;
    let mut reader = BufReader::new(stream.try_clone()?);
    loop {
        let mut request = String::new();
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => return Ok(()),
                Ok(_) if line == "\r\n" || line == "\n" => break,
                Ok(_) => request.push_str(&line),
                Err(error) => return Err(error.into()),
            }
        }
        if request.is_empty() {
            continue;
        }
        let request_target = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("/");
        let path = request_path(request_target);
        let relative = relative_file_path(path);
        if relative
            .split(['/', '\\'])
            .any(|part| part == ".." || part.contains(':'))
        {
            write!(
                stream,
                "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )?;
            return Ok(());
        }
        let file_path = root.join(relative);
        let (status, body, content_type) = match fs::read(&file_path) {
            Ok(body) => ("200 OK", body, content_type(&file_path)),
            Err(_) if should_fallback_to_index(path) => {
                let index_path = root.join("index.html");
                (
                    "200 OK",
                    fs::read(&index_path)
                        .with_context(|| format!("reading {}", index_path.display()))?,
                    content_type(&index_path),
                )
            }
            Err(_) => ("404 Not Found", b"not found".to_vec(), "text/plain"),
        };
        let close = request_wants_connection_close(&request);
        let connection_header = if close {
            "Connection: close\r\n"
        } else {
            "Connection: keep-alive\r\n"
        };
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nCross-Origin-Embedder-Policy: require-corp\r\nCross-Origin-Opener-Policy: same-origin\r\nCross-Origin-Resource-Policy: same-origin\r\n{connection_header}\r\n",
            body.len()
        )?;
        stream.write_all(&body)?;
        stream.flush()?;
        if close {
            return Ok(());
        }
    }
}

fn request_wants_connection_close(request: &str) -> bool {
    request.lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.trim().eq_ignore_ascii_case("connection")
                && value.trim().eq_ignore_ascii_case("close")
        })
    })
}

fn request_path(request_target: &str) -> &str {
    request_target
        .split_once('?')
        .map_or(request_target, |(path, _)| path)
}

fn relative_file_path(path: &str) -> &str {
    if path == "/" {
        "index.html"
    } else {
        path.trim_start_matches('/')
    }
}

fn should_fallback_to_index(path: &str) -> bool {
    let relative = path.trim_start_matches('/');
    path == "/" || Path::new(relative).extension().is_none()
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(OsStr::to_str).unwrap_or_default() {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playwright_feature_list_names_the_served_build_features() {
        let selected = AppFeatureSelection {
            features: vec!["rayx_diagnostics,indexeddb-storage-fixture".to_string()],
            ..AppFeatureSelection::default()
        };
        assert_eq!(
            playwright_feature_list(&selected),
            "indexeddb-storage-fixture,rayx_diagnostics"
        );
        let all = AppFeatureSelection {
            all_features: true,
            ..AppFeatureSelection::default()
        };
        assert_eq!(playwright_feature_list(&all), "*");
    }

    #[test]
    fn wasm_host_uses_an_explicit_deployment_base_for_deep_links()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!(
            "rayx-wasm-host-base-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        fs::create_dir_all(&root)?;

        write_wasm_host(&root, "rayx_lab_app_web", "RayX Lab", false)?;
        let html = fs::read_to_string(root.join("index.html"))?;

        assert!(html.contains(r#"<base href="/" />"#));

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn wasm_app_content_is_beside_index_and_outside_asset_catalog()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!(
            "rayx-wasm-package-assets-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        let app_root = root.join("app");
        let theme_root = root.join("theme");
        fs::create_dir_all(app_root.join("assets/icons"))?;
        fs::create_dir_all(theme_root.join("assets/icons"))?;
        let fonts_root = root.join("gpux-fonts");
        fs::create_dir_all(fonts_root.join("assets/fonts"))?;
        fs::write(fonts_root.join("assets/fonts/Fixture.txt"), "font")?;
        fs::write(fonts_root.join("fixture.rs"), "pub fn fonts() {}")?;
        fs::write(
            fonts_root.join("Cargo.toml"),
            "[package]\nname = \"gpux-fonts\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[lib]\npath = \"fixture.rs\"\n",
        )?;
        fs::write(app_root.join("fixture.rs"), "pub fn app() {}")?;
        fs::write(theme_root.join("fixture.rs"), "pub fn theme() {}")?;
        fs::write(app_root.join("assets/icons/app.svg"), "<svg/>")?;
        fs::write(theme_root.join("assets/icons/package-only.svg"), "<svg/>")?;
        fs::write(
            app_root.join("Cargo.toml"),
            r#"[package]
name = "wasm-fixture-app"
version = "0.1.0"
edition = "2024"

[lib]
path = "fixture.rs"

[dependencies]
wasm-fixture-theme = { path = "../theme" }
gpux-fonts = { path = "../gpux-fonts" }
"#,
        )?;
        fs::write(
            theme_root.join("Cargo.toml"),
            r#"[package]
name = "wasm-fixture-theme"
version = "0.1.0"
edition = "2024"

[package.metadata.rayx.assets]
manifest = "rayx.assets.toml"

[lib]
path = "fixture.rs"
"#,
        )?;
        fs::write(
            app_root.join("rayx.assets.toml"),
            r#"version = 1

[package_assets]
mode = "explicit"
include = ["wasm-fixture-theme"]

[assets]
roots = ["assets"]

[app_content]
files = ["app_settings.json"]
"#,
        )?;
        fs::write(app_root.join("app_settings.json"), "{\"wasm\":true}")?;
        fs::write(
            theme_root.join("rayx.assets.toml"),
            r#"version = 1

[asset_package]
id = "wasm-fixture-theme"
kind = "theme"
namespace = ""

[assets]
roots = [{ path = "assets/icons", mount = "icons" }]
"#,
        )?;

        let app =
            crate::app::test_support::resolve_str(app_root.to_str().expect("UTF-8 fixture path"))?;
        let assets_dir = copy_app_assets(&app, &AppFeatureSelection::default(), &root.join("out"))?;
        assets::package_app_content(&app, &root.join("out"))?;
        assert!(assets_dir.join("icons/app.svg").is_file());
        assert!(assets_dir.join("icons/package-only.svg").is_file());
        assert!(assets_dir.join("fonts/Fixture.txt").is_file());
        assert_eq!(
            fs::read_to_string(root.join("out/app_settings.json"))?,
            "{\"wasm\":true}"
        );
        assert!(!assets_dir.join("app_settings.json").exists());
        let sidecar: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(assets_dir.join("index.entries.json"))?)?;
        assert!(
            sidecar["dict"]["packageId"]
                .as_array()
                .is_some_and(|ids| ids.iter().any(|id| id == "wasm-fixture-theme"))
        );

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn request_path_strips_query_string() {
        assert_eq!(
            request_path("/stories/button?tab=usage&rayxBridge=1"),
            "/stories/button"
        );
    }

    #[test]
    fn extensionless_routes_fallback_to_index() {
        assert!(should_fallback_to_index("/stories/button"));
        assert!(should_fallback_to_index("/stories/services/storage"));
        assert!(!should_fallback_to_index("/assets/index.json"));
        assert!(!should_fallback_to_index("/rayx_lab_app_web.js"));
    }

    #[test]
    fn wasm_host_resolves_app_content_from_root_on_deep_links() -> Result<()> {
        let root = std::env::temp_dir().join(format!(
            "rayx-wasm-host-base-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        fs::create_dir_all(&root)?;

        write_wasm_host(&root, "fixture", "Fixture", false)?;
        let html = fs::read_to_string(root.join("index.html"))?;

        assert!(html.contains(r#"<base href="/" />"#));

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn playwright_test_flag_selects_one_spec_file() -> Result<()> {
        let mut args = vec![
            "--playwright-test".to_string(),
            "test_suite/rayx-component-diagnostics.spec.ts".to_string(),
            "--headed".to_string(),
        ];

        assert_eq!(
            take_playwright_tests(&mut args)?,
            vec!["test_suite/rayx-component-diagnostics.spec.ts".to_string()]
        );
        assert_eq!(args, vec!["--headed".to_string()]);

        let mut args = vec![
            "--playwright-test".to_string(),
            "test_suite/a.spec.ts".to_string(),
            "test_suite/b.spec.ts".to_string(),
            "--headed".to_string(),
            "--playwright-test".to_string(),
            "test_suite/c.spec.ts".to_string(),
        ];
        assert_eq!(
            take_playwright_tests(&mut args)?,
            vec![
                "test_suite/a.spec.ts".to_string(),
                "test_suite/b.spec.ts".to_string(),
                "test_suite/c.spec.ts".to_string(),
            ]
        );
        assert_eq!(args, vec!["--headed".to_string()]);
        assert!(take_playwright_tests(&mut vec!["--playwright-test".to_string()]).is_err());
        Ok(())
    }

    #[test]
    fn wasm_test_prepares_before_build() {
        let phases = std::cell::RefCell::new(Vec::new());
        let output = std::env::temp_dir().join("rayx-wasm-preparation-test");

        let result = prepare_before_build(
            || {
                phases.borrow_mut().push("prepare");
                Ok(())
            },
            || {
                phases.borrow_mut().push("build");
                Ok(output.clone())
            },
        );

        assert_eq!(result.expect("prepared build"), output);
        assert_eq!(phases.into_inner(), ["prepare", "build"]);
    }

    #[test]
    fn wasm_test_does_not_build_when_preparation_fails() {
        let mut built = false;

        let result = prepare_before_build(
            || Err(anyhow!("preparation failed")),
            || {
                built = true;
                Ok(PathBuf::new())
            },
        );

        assert!(result.is_err());
        assert!(!built);
    }

    #[test]
    fn cargo_feature_flags_select_an_explicit_wasm_app_profile() -> Result<()> {
        let mut args = vec![
            "--no-default-features".to_string(),
            "--features".to_string(),
            "code-editor-story,code-view-story".to_string(),
            "--features".to_string(),
            "code-editor-story".to_string(),
        ];

        let selection = AppFeatureSelection::take_from_args(&mut args)?;

        assert!(selection.no_default_features);
        assert_eq!(
            selection.normalized_features(),
            vec!["code-editor-story", "code-view-story"]
        );
        assert!(args.is_empty());
        Ok(())
    }

    #[test]
    fn wasm_diagnostic_test_selection_adds_the_diagnostic_feature() {
        let selected = wasm_test_features(true, &AppFeatureSelection::default());

        assert_eq!(selected.normalized_features(), ["rayx_diagnostics"]);
        assert!(diagnostics_enabled(&selected));
    }

    #[test]
    fn all_features_selects_a_diagnostics_enabled_wasm_host() -> Result<()> {
        let root = std::env::temp_dir().join(format!(
            "rayx-wasm-all-features-diagnostics-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        fs::create_dir_all(&root)?;
        let requested = AppFeatureSelection {
            all_features: true,
            ..AppFeatureSelection::default()
        };

        let selected = wasm_test_features(true, &requested);
        assert_eq!(selected, requested);
        assert!(diagnostics_enabled(&selected));

        write_wasm_host(&root, "fixture", "Fixture", diagnostics_enabled(&selected))?;
        let html = fs::read_to_string(root.join("index.html"))?;
        assert!(html.contains("__rayxHtmlIslandDiagnostics"));
        assert!(html.contains("wasm-thread-status"));
        assert!(html.contains("webgpu-status"));

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn production_wasm_selection_leaves_diagnostics_disabled() {
        let selected = wasm_test_features(false, &AppFeatureSelection::default());

        assert_eq!(selected, AppFeatureSelection::default());
        assert!(!diagnostics_enabled(&selected));
    }

    #[test]
    fn client_disconnects_are_not_fatal_server_errors() {
        for kind in [
            std::io::ErrorKind::BrokenPipe,
            std::io::ErrorKind::ConnectionAborted,
            std::io::ErrorKind::ConnectionReset,
            std::io::ErrorKind::UnexpectedEof,
        ] {
            let error: anyhow::Error = std::io::Error::from(kind).into();
            assert!(is_client_disconnect(&error));
        }

        let error: anyhow::Error =
            std::io::Error::from(std::io::ErrorKind::PermissionDenied).into();
        assert!(!is_client_disconnect(&error));
    }

    #[test]
    fn wasm_server_keeps_http_11_connections_alive_unless_client_closes() {
        assert!(!request_wants_connection_close(
            "GET /assets/index.json HTTP/1.1\r\nHost: localhost\r\n\r\n"
        ));
        assert!(request_wants_connection_close(
            "GET /assets/index.json HTTP/1.1\r\nConnection: close\r\n\r\n"
        ));
        assert!(request_wants_connection_close(
            "GET / HTTP/1.1\r\nconnection: CLOSE\r\n\r\n"
        ));
    }

    #[test]
    fn wasm_server_frames_multiple_requests_on_one_connection()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        use std::io::Read as _;

        fn read_response(
            reader: &mut BufReader<TcpStream>,
        ) -> std::result::Result<Vec<u8>, Box<dyn std::error::Error>> {
            let mut status = String::new();
            reader.read_line(&mut status)?;
            assert!(status.starts_with("HTTP/1.1 200 OK"));
            let mut content_length = None;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line)?;
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.strip_prefix("Content-Length:") {
                    content_length = Some(value.trim().parse::<usize>()?);
                }
            }
            let mut body = vec![0; content_length.expect("response content length")];
            reader.read_exact(&mut body)?;
            Ok(body)
        }

        let root = std::env::temp_dir().join(format!(
            "rayx-wasm-server-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        fs::create_dir_all(&root)?;
        fs::write(root.join("first.txt"), "first")?;
        fs::write(root.join("second.txt"), "second")?;

        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let address = listener.local_addr()?;
        let server_root = root.clone();
        let server = thread::spawn(move || -> Result<()> {
            let (stream, _) = listener.accept()?;
            handle_connection(stream, &server_root)
        });

        let mut client = TcpStream::connect(address)?;
        let mut reader = BufReader::new(client.try_clone()?);
        client.write_all(b"GET /first.txt HTTP/1.1\r\nHost: localhost\r\n\r\n")?;
        assert_eq!(read_response(&mut reader)?, b"first");
        client.write_all(
            b"GET /second.txt HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )?;
        assert_eq!(read_response(&mut reader)?, b"second");

        server.join().expect("WASM server test thread")?;
        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn the_server_refuses_paths_that_climb_out_of_the_output_directory() -> Result<()> {
        let parent = std::env::temp_dir().join(format!(
            "rayx-wasm-server-escape-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        let root = parent.join("out");
        fs::create_dir_all(&root)?;
        fs::write(root.join("index.html"), "index")?;
        fs::write(parent.join("secret.txt"), "secret")?;

        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let address = listener.local_addr()?;
        let server_root = root.clone();
        let server = thread::spawn(move || -> Result<()> {
            let (stream, _) = listener.accept()?;
            handle_connection(stream, &server_root)
        });

        let mut client = TcpStream::connect(address)?;
        client.write_all(
            b"GET /../secret.txt HTTP/1.1
Host: localhost
Connection: close

",
        )?;
        let mut response = String::new();
        std::io::Read::read_to_string(&mut client, &mut response)?;

        assert!(response.starts_with("HTTP/1.1 400"), "{response}");
        assert!(!response.contains("secret"), "{response}");
        server.join().expect("WASM server test thread")?;
        fs::remove_dir_all(parent).ok();
        Ok(())
    }

    #[test]
    fn html_island_diagnostics_bootstrap_includes_probe_contract() {
        let script = html_island_diagnostics_bootstrap();
        assert!(script.contains("__rayxHtmlIslandDiagnostics"));
        assert!(script.contains("__rayxHtmlIslandEnabled"));
        assert!(script.contains("htmlIslandSearch.get('rayx-html-island')"));
        assert!(script.contains("available: false"));
        assert!(script.contains("probes:"));
        assert!(script.contains("canvasLayoutSubtree,"));
        assert!(script.contains("gpuQueueCopyElementImageToTexture:"));
        assert!(script.contains("layoutSubtree: probes.canvasLayoutSubtree !== null"));
        assert!(script.contains(
            "copyElementImageToTexture: probes.gpuQueueCopyElementImageToTexture !== null"
        ));
        assert!(script.contains("textureReallocationCount: 0"));
        assert!(script.contains("path: 'unsupported'"));
    }

    #[test]
    fn generated_wasm_host_diagnostics_policy_omits_diagnostics_for_production() -> Result<()> {
        let root = std::env::temp_dir().join("rayx-wasm-host-diagnostics-policy");
        fs::create_dir_all(&root)?;

        write_wasm_host(&root, "fixture", "Fixture", false)?;
        let production = fs::read_to_string(root.join("index.html"))?;
        assert!(!production.contains("__rayx"));
        assert!(!production.contains("wasm-thread-status"));
        assert!(!production.contains("webgpu-status"));

        write_wasm_host(&root, "fixture", "Fixture", true)?;
        let diagnostics = fs::read_to_string(root.join("index.html"))?;
        assert!(diagnostics.contains("__rayxHtmlIslandDiagnostics"));
        assert!(diagnostics.contains("wasm-thread-status"));
        assert!(diagnostics.contains("webgpu-status"));

        fs::remove_dir_all(root).ok();
        Ok(())
    }
}
