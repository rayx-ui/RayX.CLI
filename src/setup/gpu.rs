//! The GPU set: detect every adapter with the operating system's own facilities (no GPU library
//! is linked into the CLI) and make sure each has a hardware driver, so native windows and
//! browser WebGPU run on hardware.
//!
//! Detection: DXGI on Windows, `lspci` plus `vulkaninfo --summary` on Linux, the Windows host's
//! adapters through `powershell.exe` under WSL, and `system_profiler SPDisplaysDataType -json`
//! on macOS.
//!
//! Driver packages, checked with `winget show` and `winget search` on 2026-10-08:
//! - NVIDIA: no winget package installs the GeForce/Studio display driver (`Nvidia.GeForceNow`,
//!   `Nvidia.CUDA` and similar are not display drivers), so the driver comes from Windows Update.
//! - AMD: no Radeon driver package (`AMD.AMDSoftwareCloudEdition` is a cloud-GPU build), so the
//!   driver comes from Windows Update.
//! - Intel: `Intel.IntelDriverAndSupportAssistant` exists and installs Intel's official driver
//!   assistant; the graphics driver also arrives through Windows Update.
//! - Qualcomm Adreno (Windows ARM64): no package; Windows Update.

use crate::host::{CommandSpec, Os, Privilege};

use super::{Action, Cx, PlanEnv, Probed, Set, SetupError, Step};

/// The GPU vendors `rayx` knows how to set up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vendor {
    Nvidia,
    Amd,
    Intel,
    Qualcomm,
    Apple,
    Microsoft,
    Other,
}

impl Vendor {
    /// From a PCI vendor id.
    pub fn from_pci(id: u32) -> Self {
        match id {
            0x10DE => Vendor::Nvidia,
            0x1002 | 0x1022 => Vendor::Amd,
            0x8086 => Vendor::Intel,
            0x5143 => Vendor::Qualcomm,
            0x106B => Vendor::Apple,
            0x1414 => Vendor::Microsoft,
            _ => Vendor::Other,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Vendor::Nvidia => "NVIDIA",
            Vendor::Amd => "AMD",
            Vendor::Intel => "Intel",
            Vendor::Qualcomm => "Qualcomm",
            Vendor::Apple => "Apple",
            Vendor::Microsoft => "Microsoft",
            Vendor::Other => "other",
        }
    }
}

/// A display adapter of this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuAdapter {
    pub vendor: Vendor,
    pub vendor_id: u32,
    pub name: String,
    pub driver_version: Option<String>,
    pub driver_date: Option<String>,
    /// Served by the Microsoft Basic Render Driver or a CPU rasterizer (lavapipe): no hardware
    /// driver.
    pub software: bool,
}

impl GpuAdapter {
    pub fn hardware(vendor_id: u32, name: &str, driver_version: Option<&str>) -> Self {
        Self {
            vendor: Vendor::from_pci(vendor_id),
            vendor_id,
            name: name.to_string(),
            driver_version: driver_version.map(str::to_string),
            driver_date: None,
            software: false,
        }
    }

    pub fn software(name: &str) -> Self {
        Self {
            vendor: Vendor::Microsoft,
            vendor_id: 0x1414,
            name: name.to_string(),
            driver_version: None,
            driver_date: None,
            software: true,
        }
    }
}

/// Whether a hardware GPU can actually be used from here: on Linux an adapter counts only when
/// Vulkan shows a non-CPU device of the same vendor, under WSL the Windows host's adapter must
/// have a driver and `/dev/dxg` must exist, and on Windows the adapter needs a driver.
pub fn hardware_adapter_usable(cx: &mut Cx, adapters: &[GpuAdapter]) -> bool {
    let env = cx.env;
    match env.host.os {
        Os::Windows => adapters
            .iter()
            .any(|a| !a.software && a.driver_version.is_some()),
        Os::MacOs => adapters.iter().any(|a| !a.software),
        Os::Linux if env.host.wsl => {
            cx.machine.exists(std::path::Path::new("/dev/dxg"))
                && adapters
                    .iter()
                    .any(|a| !a.software && a.driver_version.is_some())
        }
        Os::Linux => {
            let vulkan = linux_vulkan_devices(cx);
            adapters
                .iter()
                .any(|a| !a.software && vulkan.iter().any(|d| d.vendor_id == a.vendor_id && !d.cpu))
        }
    }
}

/// Whether any adapter is a usable hardware GPU.
pub fn has_hardware_adapter(adapters: &[GpuAdapter]) -> bool {
    adapters.iter().any(|adapter| !adapter.software)
}

/// DXGI adapter enumeration (Windows only); other systems return nothing.
pub fn dxgi_adapters() -> Vec<GpuAdapter> {
    #[cfg(windows)]
    {
        windows_dxgi::enumerate()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(windows)]
mod windows_dxgi {
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIDevice, IDXGIFactory1,
    };
    use windows::core::Interface;

    use super::{GpuAdapter, Vendor};

    pub fn enumerate() -> Vec<GpuAdapter> {
        let mut adapters = Vec::new();
        // SAFETY: plain DXGI enumeration; every returned interface is owned and released by the
        // `windows` crate wrappers.
        unsafe {
            let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else {
                return adapters;
            };
            let mut index = 0;
            while let Ok(adapter) = factory.EnumAdapters1(index) {
                index += 1;
                let Ok(desc) = adapter.GetDesc1() else {
                    continue;
                };
                let end = desc
                    .Description
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(desc.Description.len());
                let name = String::from_utf16_lossy(&desc.Description[..end]);
                let driver_version =
                    adapter
                        .CheckInterfaceSupport(&IDXGIDevice::IID)
                        .ok()
                        .map(|v| {
                            format!(
                                "{}.{}.{}.{}",
                                (v >> 48) & 0xFFFF,
                                (v >> 32) & 0xFFFF,
                                (v >> 16) & 0xFFFF,
                                v & 0xFFFF
                            )
                        });
                let software = desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0
                    || desc.VendorId == 0x1414;
                adapters.push(GpuAdapter {
                    vendor: Vendor::from_pci(desc.VendorId),
                    vendor_id: desc.VendorId,
                    name,
                    driver_version,
                    driver_date: None,
                    software,
                });
            }
        }
        adapters
    }
}

/// The adapters reported by `Get-CimInstance Win32_VideoController` as JSON (an object or an
/// array), used from WSL where DXGI is not available.
pub fn parse_windows_video_controllers(json: &str) -> Vec<GpuAdapter> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let items: Vec<serde_json::Value> = match value {
        serde_json::Value::Array(items) => items,
        object @ serde_json::Value::Object(_) => vec![object],
        _ => Vec::new(),
    };
    items
        .iter()
        .filter_map(|item| {
            let name = item["Name"].as_str()?.to_string();
            let pnp = item["PNPDeviceID"].as_str().unwrap_or_default();
            let vendor_id = pnp
                .split("VEN_")
                .nth(1)
                .and_then(|rest| rest.get(..4))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .unwrap_or(0);
            let software = name.contains("Microsoft Basic");
            Some(GpuAdapter {
                vendor: if software {
                    Vendor::Microsoft
                } else {
                    Vendor::from_pci(vendor_id)
                },
                vendor_id,
                name,
                driver_version: item["DriverVersion"].as_str().map(str::to_string),
                driver_date: item["DriverDate"].as_str().map(str::to_string),
                software,
            })
        })
        .collect()
}

/// The adapters on a Linux host: the PCI display controllers from `lspci -nn`, for example
/// `00:02.0 VGA compatible controller [0300]: Intel Corporation Alder Lake-P GT2 [Iris Xe
/// Graphics] [8086:46a6] (rev 0c)`.
pub fn parse_lspci(output: &str) -> Vec<GpuAdapter> {
    output
        .lines()
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            lower.contains("vga compatible controller")
                || lower.contains("3d controller")
                || lower.contains("display controller")
        })
        .filter_map(|line| {
            // The `[vvvv:dddd]` group after the description.
            let (ids_at, vendor_id) = line.match_indices('[').rev().find_map(|(at, _)| {
                let group = line[at + 1..].split(']').next()?;
                let (vendor, device) = group.split_once(':')?;
                (vendor.len() == 4 && device.len() == 4)
                    .then(|| u32::from_str_radix(vendor, 16).ok())
                    .flatten()
                    .map(|id| (at, id))
            })?;
            let after_class = line.split_once("]: ").map_or(line, |(_, rest)| rest);
            let offset = line.len() - after_class.len();
            let name = line
                .get(offset..ids_at)
                .unwrap_or(after_class)
                .trim()
                .to_string();
            Some(GpuAdapter::hardware(vendor_id, &name, None))
        })
        .collect()
}

/// One device of `vulkaninfo --summary`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VulkanDevice {
    pub vendor_id: u32,
    pub name: String,
    pub driver_info: Option<String>,
    /// `PHYSICAL_DEVICE_TYPE_CPU` is a software rasterizer such as lavapipe.
    pub cpu: bool,
}

pub fn parse_vulkaninfo(output: &str) -> Vec<VulkanDevice> {
    let mut devices = Vec::new();
    let mut current: Option<VulkanDevice> = None;
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("GPU") && trimmed.ends_with(':') {
            devices.extend(current.take());
            current = Some(VulkanDevice {
                vendor_id: 0,
                name: String::new(),
                driver_info: None,
                cpu: false,
            });
            continue;
        }
        let Some(device) = current.as_mut() else {
            continue;
        };
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "vendorID" => {
                device.vendor_id =
                    u32::from_str_radix(value.trim_start_matches("0x"), 16).unwrap_or(0);
            }
            "deviceName" => device.name = value.to_string(),
            "driverInfo" => device.driver_info = Some(value.to_string()),
            "deviceType" => device.cpu = value.contains("CPU"),
            _ => {}
        }
    }
    devices.extend(current);
    devices
}

/// The adapters from `system_profiler SPDisplaysDataType -json`.
pub fn parse_system_profiler(json: &str) -> Vec<GpuAdapter> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    value["SPDisplaysDataType"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let name = item["sppci_model"].as_str()?.to_string();
                    let vendor_text = item["spdisplays_vendor"].as_str().unwrap_or_default();
                    let vendor = if vendor_text.contains("Apple") || name.starts_with("Apple") {
                        (Vendor::Apple, 0x106B)
                    } else if vendor_text.to_ascii_lowercase().contains("amd") {
                        (Vendor::Amd, 0x1002)
                    } else if vendor_text.to_ascii_lowercase().contains("intel") {
                        (Vendor::Intel, 0x8086)
                    } else if vendor_text.to_ascii_lowercase().contains("nvidia") {
                        (Vendor::Nvidia, 0x10DE)
                    } else {
                        (Vendor::Other, 0)
                    };
                    Some(GpuAdapter {
                        vendor: vendor.0,
                        vendor_id: vendor.1,
                        name,
                        driver_version: item["spdisplays_mtlgpufamilysupport"]
                            .as_str()
                            .map(str::to_string),
                        driver_date: None,
                        software: false,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Every adapter on this host, found with the operating system's own facilities.
pub fn detect_adapters(cx: &mut Cx) -> Vec<GpuAdapter> {
    let env = cx.env;
    match env.host.os {
        Os::Windows => cx.machine.gpu_adapters(),
        Os::MacOs => cx
            .query(&CommandSpec::new("system_profiler").args(["SPDisplaysDataType", "-json"]))
            .filter(|outcome| outcome.is_success())
            .map(|outcome| parse_system_profiler(&outcome.stdout))
            .unwrap_or_default(),
        Os::Linux if env.host.wsl => cx
            .query(&CommandSpec::new("powershell.exe").args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-CimInstance Win32_VideoController | Select-Object Name,PNPDeviceID,DriverVersion,DriverDate | ConvertTo-Json -Compress",
            ]))
            .filter(|outcome| outcome.is_success())
            .map(|outcome| parse_windows_video_controllers(&outcome.stdout))
            .unwrap_or_default(),
        Os::Linux => {
            let pci = cx
                .query(&CommandSpec::new("lspci").arg("-nn"))
                .filter(|outcome| outcome.is_success())
                .map(|outcome| parse_lspci(&outcome.stdout))
                .unwrap_or_default();
            let vulkan = linux_vulkan_devices(cx);
            pci.into_iter()
                .map(|mut adapter| {
                    if let Some(device) = vulkan
                        .iter()
                        .find(|d| d.vendor_id == adapter.vendor_id && !d.cpu)
                    {
                        adapter.driver_version = device.driver_info.clone();
                    }
                    adapter
                })
                .collect()
        }
    }
}

fn linux_vulkan_devices(cx: &mut Cx) -> Vec<VulkanDevice> {
    cx.query(&CommandSpec::new("vulkaninfo").arg("--summary"))
        .filter(|outcome| outcome.is_success())
        .map(|outcome| parse_vulkaninfo(&outcome.stdout))
        .unwrap_or_default()
}

/// Whether `adapter` has a working hardware driver on this host.
fn has_driver(cx: &mut Cx, adapter: &GpuAdapter, linux_vulkan: &[VulkanDevice]) -> bool {
    let env = cx.env;
    if adapter.software {
        return false;
    }
    match env.host.os {
        // Windows (and the Windows host under WSL) reports the driver version of an adapter
        // that has one installed.
        Os::Windows => adapter.driver_version.is_some(),
        Os::Linux if env.host.wsl => adapter.driver_version.is_some(),
        Os::MacOs => true,
        // Virtual and unknown display adapters (virtio, VMware, Hyper-V, QEMU) have no hardware
        // driver to install; they are informational.
        Os::Linux
            if !matches!(
                adapter.vendor,
                Vendor::Nvidia | Vendor::Amd | Vendor::Intel | Vendor::Qualcomm
            ) =>
        {
            true
        }
        Os::Linux => linux_vulkan
            .iter()
            .any(|device| device.vendor_id == adapter.vendor_id && !device.cpu),
    }
}

fn gpu_step(env: &PlanEnv) -> Step {
    let os = env.host.os;
    let privilege = if os == Os::Linux && !env.host.wsl {
        Privilege::Root
    } else {
        Privilege::None
    };
    let step = Step::new(
        "gpu-driver",
        Set::Gpu,
        "GPU drivers for every adapter",
        privilege,
        |cx| {
            let adapters = detect_adapters(cx);
            let vulkan = if cx.env.host.os == Os::Linux && !cx.env.host.wsl {
                linux_vulkan_devices(cx)
            } else {
                Vec::new()
            };
            if adapters.is_empty() {
                return Probed {
                    satisfied: true,
                    detail: "no GPU adapters were found".into(),
                    ..Probed::default()
                };
            }
            // A machine served only by the Basic Render Driver or a CPU rasterizer has no GPU
            // driver; a software adapter next to real ones (WARP, lavapipe) is expected.
            let only_software = adapters.iter().all(|adapter| adapter.software);
            let missing: Vec<String> = if only_software {
                adapters
                    .iter()
                    .map(|adapter| adapter.name.clone())
                    .collect()
            } else {
                adapters
                    .iter()
                    .filter(|adapter| !adapter.software && !has_driver(cx, adapter, &vulkan))
                    .map(|adapter| adapter.name.clone())
                    .collect()
            };
            if missing.is_empty() {
                let names: Vec<&str> = adapters.iter().map(|a| a.name.as_str()).collect();
                Probed {
                    satisfied: true,
                    detail: names.join(", "),
                    ..Probed::default()
                }
            } else {
                Probed::missing_items(missing)
            }
        },
        |cx, _| {
            let adapters = detect_adapters(cx);
            let env = cx.env;
            let only_software = adapters.iter().all(|adapter| adapter.software);
            let mut actions: Vec<Action> = Vec::new();
            let mut seen: Vec<Vendor> = Vec::new();
            for adapter in &adapters {
                if seen.contains(&adapter.vendor) || (adapter.software && !only_software) {
                    continue;
                }
                seen.push(adapter.vendor);
                match (env.host.os, env.host.wsl) {
                    (Os::Windows, _) | (Os::Linux, true) => {
                        actions.extend(windows_driver_actions(cx, adapter.vendor, env.host.wsl));
                    }
                    (Os::Linux, false) => {
                        // AMD and Intel run on Mesa (`mesa-vulkan-drivers`, in the base set).
                        if adapter.vendor == Vendor::Nvidia {
                            let ubuntu = env.host.distro.as_ref().is_some_and(|d| d.id == "ubuntu");
                            if !ubuntu {
                                return Err(SetupError::Prerequisite(
                                    "install the NVIDIA driver with your distribution's tooling \
                                     (on Debian: the `nvidia-driver` package), then restart"
                                        .to_string(),
                                ));
                            }
                            actions.push(Action::Run(
                                CommandSpec::new("ubuntu-drivers")
                                    .arg("install")
                                    .root()
                                    .interactive(),
                            ));
                        }
                    }
                    (Os::MacOs, _) => {}
                }
            }
            Ok(actions)
        },
    );
    match (os, env.host.wsl) {
        (Os::Windows, _) | (Os::Linux, true) => step.with_reboot_notice(
            "Install the graphics driver offered on the Windows Update optional-updates page that \
             opened (or by the vendor's assistant), restart if asked, then run `rayx doctor`.",
        ),
        (Os::Linux, false) => {
            step.with_reboot_notice("If a driver was installed, restart for it to take effect.")
        }
        (Os::MacOs, _) => step,
    }
}

/// The Windows-side fix for one vendor. Under WSL the commands run on the Windows host through
/// interop.
fn windows_driver_actions(cx: &Cx, vendor: Vendor, wsl: bool) -> Vec<Action> {
    let mut actions = Vec::new();
    if vendor == Vendor::Intel {
        let mut install = CommandSpec::new(if wsl { "winget.exe" } else { "winget" }).args([
            "install",
            "--id",
            "Intel.IntelDriverAndSupportAssistant",
            "--exact",
        ]);
        if cx.env.yes {
            install = install.args(["--accept-package-agreements", "--accept-source-agreements"]);
        }
        actions.push(Action::Run(
            install.also_ok(super::WINGET_NOTHING_TO_DO).interactive(),
        ));
    }
    // Windows Update serves the driver for every vendor: scan, then open the optional updates.
    let (shell, flag) = if wsl {
        ("powershell.exe", "-Command")
    } else {
        ("powershell", "-Command")
    };
    actions.push(Action::Run(
        CommandSpec::new(shell)
            .args(["-NoProfile", "-NonInteractive", flag])
            .arg("Start-Process -FilePath UsoClient.exe -ArgumentList StartScan")
            .interactive(),
    ));
    actions.push(Action::Run(
        CommandSpec::new(shell)
            .args(["-NoProfile", "-NonInteractive", flag])
            .arg("Start-Process ms-settings:windowsupdate-optionalupdates")
            .interactive(),
    ));
    actions
}

fn vulkan_tools_step() -> Step {
    Step::apt(
        "vulkan-tools",
        Set::Gpu,
        "vulkaninfo (vulkan-tools) and lspci (pciutils) for the adapter probe",
        &["vulkan-tools", "pciutils"],
    )
}

pub fn steps(env: &PlanEnv) -> Result<Vec<Step>, SetupError> {
    let mut steps = Vec::new();
    if env.host.os == Os::Linux && !env.host.wsl {
        steps.push(vulkan_tools_step());
    }
    steps.push(gpu_step(env));
    Ok(steps)
}
