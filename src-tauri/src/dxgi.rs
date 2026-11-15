//! Screens as Desktop Duplication numbers them, with their HDR state.

use windows::core::Interface;
use windows::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig,
    DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
    DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SDR_WHITE_LEVEL, DISPLAYCONFIG_SOURCE_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::Graphics::Dxgi::Common::DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020;
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, IDXGIOutput6};

pub struct Output {
    /// `output_idx` for ffmpeg's `ddagrab`.
    pub index: usize,
    /// Top-left corner on the desktop, in pixels.
    pub x: i32,
    pub y: i32,
    pub hdr: bool,
    /// Where SDR content sits on this screen, when it is in HDR mode.
    pub sdr_white_nits: f64,
}

fn wide(s: &[u16]) -> String {
    let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    String::from_utf16_lossy(&s[..end])
}

/// The "SDR content brightness" setting of each active display, by GDI
/// device name (`\\.\DISPLAY1`).
fn sdr_white_levels() -> Vec<(String, f64)> {
    let mut out = Vec::new();
    unsafe {
        let (mut n_paths, mut n_modes) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut n_paths, &mut n_modes)
            != ERROR_SUCCESS
        {
            return out;
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); n_paths as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); n_modes as usize];
        if QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut n_paths,
            paths.as_mut_ptr(),
            &mut n_modes,
            modes.as_mut_ptr(),
            None,
        ) != ERROR_SUCCESS
        {
            return out;
        }
        for path in &paths[..n_paths as usize] {
            let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                    size: std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                    adapterId: path.sourceInfo.adapterId,
                    id: path.sourceInfo.id,
                },
                ..Default::default()
            };
            if DisplayConfigGetDeviceInfo(&mut source.header) != 0 {
                continue;
            }
            let mut white = DISPLAYCONFIG_SDR_WHITE_LEVEL {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
                    size: std::mem::size_of::<DISPLAYCONFIG_SDR_WHITE_LEVEL>() as u32,
                    adapterId: path.targetInfo.adapterId,
                    id: path.targetInfo.id,
                },
                ..Default::default()
            };
            if DisplayConfigGetDeviceInfo(&mut white.header) != 0 {
                continue;
            }
            // In thousandths of 80 nits.
            let nits = white.SDRWhiteLevel as f64 / 1000.0 * 80.0;
            out.push((wide(&source.viewGdiDeviceName), nits));
        }
    }
    out
}

/// The outputs of the first graphics adapter, which is the one `ddagrab`
/// duplicates.
pub fn outputs() -> Vec<Output> {
    let whites = sdr_white_levels();
    let mut list = Vec::new();
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else {
            return list;
        };
        let Ok(adapter) = factory.EnumAdapters1(0) else {
            return list;
        };
        let mut index = 0;
        while let Ok(output) = adapter.EnumOutputs(index) {
            index += 1;
            let Ok(output) = output.cast::<IDXGIOutput6>() else {
                continue;
            };
            let Ok(desc) = output.GetDesc1() else {
                continue;
            };
            let name = wide(&desc.DeviceName);
            let white = whites
                .iter()
                .find(|(n, _)| *n == name)
                .map_or(cinesnap_core::tonemap::REFERENCE_NITS, |(_, w)| *w);
            list.push(Output {
                index: index as usize - 1,
                x: desc.DesktopCoordinates.left,
                y: desc.DesktopCoordinates.top,
                hdr: desc.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020,
                sdr_white_nits: white,
            });
        }
    }
    list
}
