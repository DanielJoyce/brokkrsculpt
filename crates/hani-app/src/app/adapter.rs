// SPDX-License-Identifier: AGPL-3.0-only

//! The graphics adapter, named on the About card.
//!
//! Left not done by the About card (2026-11-27) and asked for by the
//! maintainer among the features missed: name the graphics adapter it runs
//! on. iced chooses the adapter inside its
//! compositor and hands the viewport a device that cannot say which it
//! came from; iced's own answer (`iced::system::information`) needs its
//! `sysinfo` feature, a crate of its own and, on Windows, the `windows`
//! crate, for one line. So the adapter is asked for again the way iced
//! asks: the backends `WGPU_BACKEND` names or every one, and the power
//! preference `WGPU_POWER_PREF` names or high performance. Without a window
//! to be compatible with, a machine with two adapters could in principle be
//! given another than the window's: the line's tooltip says so.
//!
//! Asked once, when About is first opened, on the task pool, and kept.

/// The adapter a request like iced's is given: its name and the backend it
/// is driven through, or why there is none.
pub(super) async fn asked() -> String {
    let instance = wgpu::util::new_instance_with_webgpu_detection(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::from_env().unwrap_or(wgpu::Backends::all()),
        ..Default::default()
    })
    .await;
    let options = wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::from_env()
            .unwrap_or(wgpu::PowerPreference::HighPerformance),
        compatible_surface: None,
        force_fallback_adapter: false,
    };
    match instance.request_adapter(&options).await {
        Ok(adapter) => named(&adapter.get_info()),
        Err(_) => "none found".to_string(),
    }
}

/// An adapter as the card names it.
fn named(info: &wgpu::AdapterInfo) -> String {
    let name = info.name.trim();
    let name = if name.is_empty() { "unnamed adapter" } else { name };
    format!("{name} ({:?})", info.backend)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(name: &str) -> wgpu::AdapterInfo {
        wgpu::AdapterInfo {
            name: name.to_string(),
            vendor: 0,
            device: 0,
            device_type: wgpu::DeviceType::DiscreteGpu,
            driver: String::new(),
            driver_info: String::new(),
            backend: wgpu::Backend::Vulkan,
        }
    }

    /// An adapter is named by its own name and the backend it is driven
    /// through, and one with no name is still named.
    #[test]
    fn an_adapter_is_named_with_its_backend() {
        assert_eq!(named(&info("  Radeon RX 7600 ")), "Radeon RX 7600 (Vulkan)");
        assert_eq!(named(&info("")), "unnamed adapter (Vulkan)");
    }

    /// Asked on this machine, the answer is a line: an adapter's name, or
    /// that there is none. Printed, so a run with `--nocapture` says which.
    #[test]
    fn the_adapter_is_asked_for_and_answers() {
        let answer = pollster::block_on(asked());
        eprintln!("graphics adapter: {answer}");
        assert!(!answer.is_empty() && !answer.contains('\n'), "{answer:?}");
    }
}
