//! How rendered frames reach SteamVR.
//!
//! * **Windows:** a D3D11 texture on SteamVR's GPU, updated in place and shown
//!   with `SetOverlayTexture`. The compositor copies it GPU to GPU.
//! * **Everywhere else (and as a fallback):** `SetOverlayRaw`, which sends the
//!   pixels through shared memory and makes the compositor rebuild its
//!   texture on every call. That is cheap for us but can make the overlay
//!   flicker when it changes often.

use anyhow::Result;

use crate::ffi::{OverlayHandle, Session};

pub(crate) enum Uploader {
    Raw,
    #[cfg(windows)]
    Gpu(d3d11::GpuTexture),
}

impl Uploader {
    /// Picks the best available path for a `width` x `height` overlay.
    pub fn new(session: &Session<'_>, width: u32, height: u32) -> Self {
        #[cfg(windows)]
        match d3d11::GpuTexture::new(session.output_device_luid(), width, height) {
            Ok(texture) => {
                tracing::info!("Overlay frames are uploaded as a D3D11 texture");
                return Self::Gpu(texture);
            }
            Err(e) => tracing::warn!("D3D11 overlay upload unavailable ({e:#}), using raw uploads"),
        }
        #[cfg(not(windows))]
        let _ = (session, width, height);
        tracing::info!("Overlay frames are uploaded as raw pixels");
        Self::Raw
    }

    pub fn upload(
        &mut self,
        session: &Session<'_>,
        overlay: OverlayHandle,
        pixels: &mut [u8],
        width: u32,
        height: u32,
    ) -> Result<()> {
        match self {
            Self::Raw => session.set_overlay_raw(overlay, pixels, width, height),
            #[cfg(windows)]
            Self::Gpu(texture) => {
                let result = texture.upload(session, overlay, pixels);
                if let Err(e) = result {
                    tracing::warn!("Texture upload failed ({e:#}), switching to raw uploads");
                    *self = Self::Raw;
                    return session.set_overlay_raw(overlay, pixels, width, height);
                }
                Ok(())
            }
        }
    }
}

#[cfg(windows)]
pub(crate) mod d3d11 {
    use anyhow::{anyhow, bail, Context, Result};
    use windows::core::Interface;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN};
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
        D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_FLAG, D3D11_RESOURCE_MISC_SHARED,
        D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    };
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC};
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory1};

    use crate::ffi::{OverlayHandle, Session, VrTexture, COLOR_SPACE_GAMMA, TEXTURE_TYPE_DIRECTX};

    /// A texture that is rewritten for every changed frame.
    pub struct GpuTexture {
        _device: ID3D11Device,
        context: ID3D11DeviceContext,
        texture: ID3D11Texture2D,
        width: u32,
        height: u32,
    }

    /// The adapter with the given LUID (`None` for 0 or if it is not found, in
    /// which case the default adapter is used).
    fn find_adapter(luid: u64) -> Result<Option<IDXGIAdapter>> {
        if luid == 0 {
            return Ok(None);
        }
        // SAFETY: plain DXGI enumeration.
        unsafe {
            let factory: IDXGIFactory1 = CreateDXGIFactory1().context("CreateDXGIFactory1")?;
            let mut index = 0;
            while let Ok(adapter) = factory.EnumAdapters1(index) {
                let desc = adapter.GetDesc1().context("GetDesc1")?;
                let found = ((desc.AdapterLuid.HighPart as u32 as u64) << 32)
                    | u64::from(desc.AdapterLuid.LowPart);
                if found == luid {
                    return Ok(Some(adapter.cast().context("cast to IDXGIAdapter")?));
                }
                index += 1;
            }
        }
        tracing::warn!("SteamVR's GPU (LUID {luid:#x}) was not found, using the default GPU");
        Ok(None)
    }

    impl GpuTexture {
        pub fn new(luid: u64, width: u32, height: u32) -> Result<Self> {
            Self::with_options(luid, width, height, true)
        }

        pub fn with_options(luid: u64, width: u32, height: u32, shared: bool) -> Result<Self> {
            let adapter = find_adapter(luid)?;
            let driver = if adapter.is_some() {
                D3D_DRIVER_TYPE_UNKNOWN
            } else {
                D3D_DRIVER_TYPE_HARDWARE
            };
            let mut device = None;
            let mut context = None;
            // SAFETY: out pointers are valid for the duration of the call.
            unsafe {
                D3D11CreateDevice(
                    adapter.as_ref(),
                    driver,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_FLAG(0),
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut context),
                )
                .context("D3D11CreateDevice")?;
            }
            let device = device.ok_or_else(|| anyhow!("no D3D11 device"))?;
            let context = context.ok_or_else(|| anyhow!("no D3D11 context"))?;

            let desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                CPUAccessFlags: 0,
                MiscFlags: if shared {
                    D3D11_RESOURCE_MISC_SHARED.0 as u32
                } else {
                    0
                },
            };
            let mut texture = None;
            // SAFETY: valid descriptor and out pointer.
            unsafe {
                device
                    .CreateTexture2D(&desc, None, Some(&mut texture))
                    .context("CreateTexture2D")?;
            }
            Ok(Self {
                _device: device,
                context,
                texture: texture.ok_or_else(|| anyhow!("no texture"))?,
                width,
                height,
            })
        }

        /// Copies `rgba` into the texture and hands it to SteamVR.
        pub fn upload(
            &self,
            session: &Session<'_>,
            overlay: OverlayHandle,
            rgba: &[u8],
        ) -> Result<()> {
            if rgba.len() != self.width as usize * self.height as usize * 4 {
                bail!("frame size does not match the texture");
            }
            // SAFETY: `rgba` holds `height` rows of `width * 4` bytes.
            unsafe {
                self.context.UpdateSubresource(
                    &self.texture,
                    0,
                    None,
                    rgba.as_ptr().cast(),
                    self.width * 4,
                    0,
                );
                // Submit the copy so the compositor sees finished pixels.
                self.context.Flush();
            }
            let texture = VrTexture {
                handle: self.texture.as_raw(),
                texture_type: TEXTURE_TYPE_DIRECTX,
                color_space: COLOR_SPACE_GAMMA,
            };
            // SAFETY: a live D3D11 texture on SteamVR's adapter.
            unsafe { session.set_overlay_texture(overlay, &texture) }
        }
    }
}
