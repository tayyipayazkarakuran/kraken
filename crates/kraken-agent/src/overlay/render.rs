//! A small D3D11 renderer for egui meshes, run inside `Present`. The game's
//! pipeline state is saved before drawing and restored afterwards.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use kraken_proto::{UiInput, UiMesh, UiTexture};
use parking_lot::Mutex;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{
    D3D_PRIMITIVE_TOPOLOGY, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, ID3DBlob,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R32_UINT, DXGI_FORMAT_R32G32_FLOAT, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::IDXGISwapChain;
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
use windows::core::{Interface, PCSTR, s};

const SHADER: &str = r"
cbuffer Screen : register(b0) { float2 screen; float2 pad; };
struct VsIn { float2 pos : POSITION; float2 uv : TEXCOORD; float4 color : COLOR; };
struct PsIn { float4 pos : SV_POSITION; float2 uv : TEXCOORD; float4 color : COLOR; };
PsIn vs(VsIn i) {
    PsIn o;
    o.pos = float4(i.pos.x / screen.x * 2.0 - 1.0, 1.0 - i.pos.y / screen.y * 2.0, 0.0, 1.0);
    o.uv = i.uv;
    o.color = i.color;
    return o;
}
Texture2D tex : register(t0);
SamplerState smp : register(s0);
float4 ps(PsIn i) : SV_Target { return i.color * tex.Sample(smp, i.uv); }
";

/// How often the client size is re-sent, so a reconnected app learns it.
const RESIZE_EVERY_MS: u128 = 1_000;

#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    pos: [f32; 2],
    uv: [f32; 2],
    color: [u8; 4],
}

struct Texture {
    texture: ID3D11Texture2D,
    view: ID3D11ShaderResourceView,
    width: u32,
    height: u32,
    linear: bool,
}

struct Batch {
    texture: u64,
    clip: [f32; 4],
    start: u32,
    count: u32,
    base: i32,
}

struct Renderer {
    swap: usize,
    chain: IDXGISwapChain,
    hwnd: HWND,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    layout: ID3D11InputLayout,
    constants: ID3D11Buffer,
    linear: ID3D11SamplerState,
    nearest: ID3D11SamplerState,
    blend: ID3D11BlendState,
    raster: ID3D11RasterizerState,
    depth: ID3D11DepthStencilState,
    vertices: Option<(ID3D11Buffer, usize)>,
    indices: Option<(ID3D11Buffer, usize)>,
    textures: HashMap<u64, Texture>,
    target: Option<ID3D11RenderTargetView>,
    version: u64,
    batches: Vec<Batch>,
    frame: (f32, f32),
    sent_size: (u32, u32),
    sent_at: Option<Instant>,
}

unsafe impl Send for Renderer {}

static RENDERER: Mutex<Option<Renderer>> = Mutex::new(None);
static FAILED: AtomicBool = AtomicBool::new(false);

fn blob(blob: &ID3DBlob) -> &[u8] {
    unsafe { std::slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize()) }
}

fn compile(entry: PCSTR, target: PCSTR) -> Result<ID3DBlob, String> {
    let mut code = None;
    let mut errors = None;
    let result = unsafe {
        D3DCompile(
            SHADER.as_ptr().cast(),
            SHADER.len(),
            s!("shader"),
            None,
            None,
            entry,
            target,
            0,
            0,
            &mut code,
            Some(&mut errors),
        )
    };
    if let Err(error) = result {
        let detail = errors
            .as_ref()
            .map(|e| String::from_utf8_lossy(blob(e)).into_owned())
            .unwrap_or_default();
        return Err(format!("shader: {error} {detail}"));
    }
    code.ok_or_else(|| "shader: no code".into())
}

fn check<T>(value: Option<T>, what: &str) -> Result<T, String> {
    value.ok_or_else(|| format!("{what} not created"))
}

impl Renderer {
    fn new(this: *mut c_void) -> Result<Self, String> {
        unsafe {
            let swap = IDXGISwapChain::from_raw_borrowed(&this).ok_or("no swap chain")?;
            let device: ID3D11Device = swap.GetDevice().map_err(|e| format!("device: {e}"))?;
            let context = device
                .GetImmediateContext()
                .map_err(|e| format!("context: {e}"))?;
            let desc = swap.GetDesc().map_err(|e| format!("swap desc: {e}"))?;

            let vs_code = compile(s!("vs"), s!("vs_4_0"))?;
            let ps_code = compile(s!("ps"), s!("ps_4_0"))?;
            let mut vs = None;
            device
                .CreateVertexShader(blob(&vs_code), None, Some(&mut vs))
                .map_err(|e| format!("vertex shader: {e}"))?;
            let mut ps = None;
            device
                .CreatePixelShader(blob(&ps_code), None, Some(&mut ps))
                .map_err(|e| format!("pixel shader: {e}"))?;
            let elements = [
                D3D11_INPUT_ELEMENT_DESC {
                    SemanticName: s!("POSITION"),
                    SemanticIndex: 0,
                    Format: DXGI_FORMAT_R32G32_FLOAT,
                    InputSlot: 0,
                    AlignedByteOffset: 0,
                    InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                    InstanceDataStepRate: 0,
                },
                D3D11_INPUT_ELEMENT_DESC {
                    SemanticName: s!("TEXCOORD"),
                    SemanticIndex: 0,
                    Format: DXGI_FORMAT_R32G32_FLOAT,
                    InputSlot: 0,
                    AlignedByteOffset: 8,
                    InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                    InstanceDataStepRate: 0,
                },
                D3D11_INPUT_ELEMENT_DESC {
                    SemanticName: s!("COLOR"),
                    SemanticIndex: 0,
                    Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    InputSlot: 0,
                    AlignedByteOffset: 16,
                    InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                    InstanceDataStepRate: 0,
                },
            ];
            let mut layout = None;
            device
                .CreateInputLayout(&elements, blob(&vs_code), Some(&mut layout))
                .map_err(|e| format!("input layout: {e}"))?;

            let mut constants = None;
            device
                .CreateBuffer(
                    &D3D11_BUFFER_DESC {
                        ByteWidth: 16,
                        Usage: D3D11_USAGE_DYNAMIC,
                        BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                        ..Default::default()
                    },
                    None,
                    Some(&mut constants),
                )
                .map_err(|e| format!("constants: {e}"))?;

            let sampler = |filter| {
                let mut out = None;
                device
                    .CreateSamplerState(
                        &D3D11_SAMPLER_DESC {
                            Filter: filter,
                            AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                            AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                            AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                            ComparisonFunc: D3D11_COMPARISON_ALWAYS,
                            MaxLOD: f32::MAX,
                            ..Default::default()
                        },
                        Some(&mut out),
                    )
                    .map(|()| out)
            };
            let linear = sampler(D3D11_FILTER_MIN_MAG_MIP_LINEAR).map_err(|e| format!("sampler: {e}"))?;
            let nearest = sampler(D3D11_FILTER_MIN_MAG_MIP_POINT).map_err(|e| format!("sampler: {e}"))?;

            let mut blend_desc = D3D11_BLEND_DESC::default();
            blend_desc.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
                BlendEnable: true.into(),
                SrcBlend: D3D11_BLEND_ONE,
                DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
                BlendOp: D3D11_BLEND_OP_ADD,
                SrcBlendAlpha: D3D11_BLEND_INV_DEST_ALPHA,
                DestBlendAlpha: D3D11_BLEND_ONE,
                BlendOpAlpha: D3D11_BLEND_OP_ADD,
                RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
            };
            let mut blend = None;
            device
                .CreateBlendState(&blend_desc, Some(&mut blend))
                .map_err(|e| format!("blend: {e}"))?;
            let mut raster = None;
            device
                .CreateRasterizerState(
                    &D3D11_RASTERIZER_DESC {
                        FillMode: D3D11_FILL_SOLID,
                        CullMode: D3D11_CULL_NONE,
                        ScissorEnable: true.into(),
                        DepthClipEnable: true.into(),
                        ..Default::default()
                    },
                    Some(&mut raster),
                )
                .map_err(|e| format!("raster: {e}"))?;
            let mut depth = None;
            device
                .CreateDepthStencilState(
                    &D3D11_DEPTH_STENCIL_DESC {
                        DepthEnable: false.into(),
                        DepthWriteMask: D3D11_DEPTH_WRITE_MASK_ALL,
                        DepthFunc: D3D11_COMPARISON_ALWAYS,
                        StencilEnable: false.into(),
                        ..Default::default()
                    },
                    Some(&mut depth),
                )
                .map_err(|e| format!("depth: {e}"))?;

            super::input::attach(desc.OutputWindow);
            Ok(Self {
                swap: this as usize,
                chain: swap.clone(),
                hwnd: desc.OutputWindow,
                device,
                context,
                vs: check(vs, "vertex shader")?,
                ps: check(ps, "pixel shader")?,
                layout: check(layout, "input layout")?,
                constants: check(constants, "constants")?,
                linear: check(linear, "sampler")?,
                nearest: check(nearest, "sampler")?,
                blend: check(blend, "blend")?,
                raster: check(raster, "raster")?,
                depth: check(depth, "depth")?,
                vertices: None,
                indices: None,
                textures: HashMap::new(),
                target: None,
                version: u64::MAX,
                batches: Vec::new(),
                frame: (1.0, 1.0),
                sent_size: (0, 0),
                sent_at: None,
            })
        }
    }

    /// Tells the app the client size when it changes, and every second.
    fn report_size(&mut self) {
        let mut rect = RECT::default();
        if unsafe { GetClientRect(self.hwnd, &mut rect) }.is_err() {
            return;
        }
        let size = (
            (rect.right - rect.left).max(0) as u32,
            (rect.bottom - rect.top).max(0) as u32,
        );
        let due = self
            .sent_at
            .is_none_or(|t| t.elapsed().as_millis() >= RESIZE_EVERY_MS);
        if size != self.sent_size || due {
            self.sent_size = size;
            self.sent_at = Some(Instant::now());
            super::send(UiInput::Resized {
                width: size.0,
                height: size.1,
            });
        }
    }

    fn update_textures(&mut self, uploads: Vec<UiTexture>, freed: Vec<u64>) {
        for id in freed {
            self.textures.remove(&id);
        }
        for upload in uploads {
            if let Err(error) = self.upload(&upload) {
                tracing::warn!(id = upload.id, %error, "overlay texture upload failed");
            }
        }
    }

    fn upload(&mut self, upload: &UiTexture) -> Result<(), String> {
        let expected = upload.width as usize * upload.height as usize * 4;
        if upload.rgba.len() != expected || upload.width == 0 || upload.height == 0 {
            return Err("bad texture size".into());
        }
        unsafe {
            if let Some([x, y]) = upload.origin {
                let texture = self.textures.get(&upload.id).ok_or("patch for unknown texture")?;
                if x + upload.width > texture.width || y + upload.height > texture.height {
                    return Err("patch outside texture".into());
                }
                let area = D3D11_BOX {
                    left: x,
                    top: y,
                    front: 0,
                    right: x + upload.width,
                    bottom: y + upload.height,
                    back: 1,
                };
                self.context.UpdateSubresource(
                    &texture.texture,
                    0,
                    Some(&area),
                    upload.rgba.as_ptr().cast(),
                    upload.width * 4,
                    0,
                );
                return Ok(());
            }
            let desc = D3D11_TEXTURE2D_DESC {
                Width: upload.width,
                Height: upload.height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                ..Default::default()
            };
            let data = D3D11_SUBRESOURCE_DATA {
                pSysMem: upload.rgba.as_ptr().cast(),
                SysMemPitch: upload.width * 4,
                SysMemSlicePitch: 0,
            };
            let mut texture = None;
            self.device
                .CreateTexture2D(&desc, Some(&data), Some(&mut texture))
                .map_err(|e| e.to_string())?;
            let texture = check(texture, "texture")?;
            let mut view = None;
            self.device
                .CreateShaderResourceView(&texture, None, Some(&mut view))
                .map_err(|e| e.to_string())?;
            self.textures.insert(
                upload.id,
                Texture {
                    texture,
                    view: check(view, "texture view")?,
                    width: upload.width,
                    height: upload.height,
                    linear: upload.linear,
                },
            );
        }
        Ok(())
    }

    /// Grows a dynamic buffer to hold `bytes`.
    fn buffer(
        &self,
        slot: &mut Option<(ID3D11Buffer, usize)>,
        bytes: usize,
        bind: D3D11_BIND_FLAG,
    ) -> Result<(), String> {
        if slot.as_ref().is_some_and(|(_, size)| *size >= bytes) {
            return Ok(());
        }
        let size = bytes.next_power_of_two().max(64 * 1024);
        let mut buffer = None;
        unsafe {
            self.device
                .CreateBuffer(
                    &D3D11_BUFFER_DESC {
                        ByteWidth: size as u32,
                        Usage: D3D11_USAGE_DYNAMIC,
                        BindFlags: bind.0 as u32,
                        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                        ..Default::default()
                    },
                    None,
                    Some(&mut buffer),
                )
                .map_err(|e| e.to_string())?;
        }
        *slot = Some((check(buffer, "buffer")?, size));
        Ok(())
    }

    fn write<T: Copy>(&self, buffer: &ID3D11Buffer, data: &[T]) -> Result<(), String> {
        unsafe {
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(buffer, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))
                .map_err(|e| e.to_string())?;
            std::ptr::copy_nonoverlapping(data.as_ptr(), mapped.pData.cast::<T>(), data.len());
            self.context.Unmap(buffer, 0);
        }
        Ok(())
    }

    /// Re-uploads the geometry when the app sent a new frame.
    fn upload_meshes(&mut self, meshes: &[UiMesh], width: u32, height: u32) -> Result<(), String> {
        let mut vertices: Vec<Vertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        self.batches.clear();
        for mesh in meshes {
            if mesh.indices.is_empty() {
                continue;
            }
            self.batches.push(Batch {
                texture: mesh.texture,
                clip: [
                    mesh.clip.min[0],
                    mesh.clip.min[1],
                    mesh.clip.max[0],
                    mesh.clip.max[1],
                ],
                start: indices.len() as u32,
                count: mesh.indices.len() as u32,
                base: vertices.len() as i32,
            });
            vertices.extend(mesh.vertices.iter().map(|v| Vertex {
                pos: v.pos,
                uv: v.uv,
                color: v.color,
            }));
            indices.extend_from_slice(&mesh.indices);
        }
        self.frame = (width.max(1) as f32, height.max(1) as f32);
        if vertices.is_empty() {
            return Ok(());
        }
        let mut slot = self.vertices.take();
        self.buffer(
            &mut slot,
            vertices.len() * size_of::<Vertex>(),
            D3D11_BIND_VERTEX_BUFFER,
        )?;
        self.vertices = slot;
        let mut slot = self.indices.take();
        self.buffer(&mut slot, indices.len() * 4, D3D11_BIND_INDEX_BUFFER)?;
        self.indices = slot;
        if let Some((buffer, _)) = &self.vertices {
            self.write(buffer, &vertices)?;
        }
        if let Some((buffer, _)) = &self.indices {
            self.write(buffer, &indices)?;
        }
        Ok(())
    }

    fn draw(&mut self) -> Result<(), String> {
        self.report_size();
        let (meshes, version, width, height, uploads, freed) = {
            let mut scene = super::SCENE.lock();
            (
                Arc::clone(&scene.meshes),
                scene.version,
                scene.width,
                scene.height,
                std::mem::take(&mut scene.textures),
                std::mem::take(&mut scene.freed),
            )
        };
        self.update_textures(uploads, freed);
        if version != self.version {
            self.version = version;
            self.upload_meshes(&meshes, width, height)?;
        }
        if self.batches.is_empty() {
            return Ok(());
        }
        let swap = self.chain.clone();
        if self.target.is_none() {
            unsafe {
                let back: ID3D11Texture2D = swap.GetBuffer(0).map_err(|e| format!("back buffer: {e}"))?;
                let mut view = None;
                self.device
                    .CreateRenderTargetView(&back, None, Some(&mut view))
                    .map_err(|e| format!("target view: {e}"))?;
                self.target = view;
            }
        }
        let desc = unsafe { swap.GetDesc() }.map_err(|e| e.to_string())?;
        let (bw, bh) = (desc.BufferDesc.Width as f32, desc.BufferDesc.Height as f32);
        let saved = State::save(&self.context);
        let result = self.render(bw, bh);
        saved.restore(&self.context);
        result
    }

    fn render(&self, bw: f32, bh: f32) -> Result<(), String> {
        let (Some((vertices, _)), Some((indices, _)), Some(target)) =
            (&self.vertices, &self.indices, &self.target)
        else {
            return Ok(());
        };
        let (fw, fh) = self.frame;
        let (sx, sy) = (bw / fw, bh / fh);
        let ctx = &self.context;
        self.write(&self.constants, &[fw, fh, 0.0, 0.0])?;
        unsafe {
            ctx.OMSetRenderTargets(Some(&[Some(target.clone())]), None);
            ctx.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: bw,
                Height: bh,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
            ctx.IASetInputLayout(&self.layout);
            let stride = size_of::<Vertex>() as u32;
            let offset = 0u32;
            ctx.IASetVertexBuffers(0, 1, Some(&Some(vertices.clone())), Some(&stride), Some(&offset));
            ctx.IASetIndexBuffer(indices, DXGI_FORMAT_R32_UINT, 0);
            ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.VSSetShader(&self.vs, None);
            ctx.VSSetConstantBuffers(0, Some(&[Some(self.constants.clone())]));
            ctx.PSSetShader(&self.ps, None);
            ctx.GSSetShader(None::<&ID3D11GeometryShader>, None);
            ctx.HSSetShader(None::<&ID3D11HullShader>, None);
            ctx.DSSetShader(None::<&ID3D11DomainShader>, None);
            ctx.CSSetShader(None::<&ID3D11ComputeShader>, None);
            ctx.OMSetBlendState(&self.blend, Some(&[0.0; 4]), 0xffff_ffff);
            ctx.OMSetDepthStencilState(&self.depth, 0);
            ctx.RSSetState(&self.raster);
            for batch in &self.batches {
                let Some(texture) = self.textures.get(&batch.texture) else {
                    continue;
                };
                let clip = RECT {
                    left: (batch.clip[0] * sx).floor().max(0.0) as i32,
                    top: (batch.clip[1] * sy).floor().max(0.0) as i32,
                    right: (batch.clip[2] * sx).ceil().min(bw) as i32,
                    bottom: (batch.clip[3] * sy).ceil().min(bh) as i32,
                };
                if clip.right <= clip.left || clip.bottom <= clip.top {
                    continue;
                }
                ctx.RSSetScissorRects(Some(&[clip]));
                ctx.PSSetShaderResources(0, Some(&[Some(texture.view.clone())]));
                let sampler = if texture.linear {
                    &self.linear
                } else {
                    &self.nearest
                };
                ctx.PSSetSamplers(0, Some(&[Some(sampler.clone())]));
                ctx.DrawIndexed(batch.count, batch.start, batch.base);
            }
        }
        Ok(())
    }
}

/// The pipeline state the overlay touches, captured and put back as is.
struct State {
    scissors: Vec<RECT>,
    viewports: Vec<D3D11_VIEWPORT>,
    raster: Option<ID3D11RasterizerState>,
    blend: Option<ID3D11BlendState>,
    blend_factor: [f32; 4],
    sample_mask: u32,
    depth: Option<ID3D11DepthStencilState>,
    stencil_ref: u32,
    ps_resource: [Option<ID3D11ShaderResourceView>; 1],
    ps_sampler: [Option<ID3D11SamplerState>; 1],
    ps: Option<ID3D11PixelShader>,
    vs: Option<ID3D11VertexShader>,
    gs: Option<ID3D11GeometryShader>,
    hs: Option<ID3D11HullShader>,
    ds: Option<ID3D11DomainShader>,
    cs: Option<ID3D11ComputeShader>,
    vs_constants: [Option<ID3D11Buffer>; 1],
    topology: D3D_PRIMITIVE_TOPOLOGY,
    index: Option<ID3D11Buffer>,
    index_format: DXGI_FORMAT,
    index_offset: u32,
    vertex: Option<ID3D11Buffer>,
    vertex_stride: u32,
    vertex_offset: u32,
    layout: Option<ID3D11InputLayout>,
    targets: [Option<ID3D11RenderTargetView>; 8],
    depth_view: Option<ID3D11DepthStencilView>,
}

impl State {
    fn save(ctx: &ID3D11DeviceContext) -> Self {
        unsafe {
            let mut count = D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE;
            let mut scissors = vec![RECT::default(); count as usize];
            ctx.RSGetScissorRects(&mut count, Some(scissors.as_mut_ptr()));
            scissors.truncate(count as usize);
            let mut count = D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE;
            let mut viewports = vec![D3D11_VIEWPORT::default(); count as usize];
            ctx.RSGetViewports(&mut count, Some(viewports.as_mut_ptr()));
            viewports.truncate(count as usize);
            let mut state = Self {
                scissors,
                viewports,
                raster: ctx.RSGetState().ok(),
                blend: None,
                blend_factor: [0.0; 4],
                sample_mask: 0,
                depth: None,
                stencil_ref: 0,
                ps_resource: [None],
                ps_sampler: [None],
                ps: None,
                vs: None,
                gs: None,
                hs: None,
                ds: None,
                cs: None,
                vs_constants: [None],
                topology: ctx.IAGetPrimitiveTopology(),
                index: None,
                index_format: DXGI_FORMAT::default(),
                index_offset: 0,
                vertex: None,
                vertex_stride: 0,
                vertex_offset: 0,
                layout: ctx.IAGetInputLayout().ok(),
                targets: Default::default(),
                depth_view: None,
            };
            ctx.OMGetBlendState(
                Some(&mut state.blend),
                Some(&mut state.blend_factor),
                Some(&mut state.sample_mask),
            );
            ctx.OMGetDepthStencilState(Some(&mut state.depth), Some(&mut state.stencil_ref));
            ctx.PSGetShaderResources(0, Some(&mut state.ps_resource));
            ctx.PSGetSamplers(0, Some(&mut state.ps_sampler));
            ctx.PSGetShader(&mut state.ps, None, None);
            ctx.VSGetShader(&mut state.vs, None, None);
            ctx.GSGetShader(&mut state.gs, None, None);
            ctx.HSGetShader(&mut state.hs, None, None);
            ctx.DSGetShader(&mut state.ds, None, None);
            ctx.CSGetShader(&mut state.cs, None, None);
            ctx.VSGetConstantBuffers(0, Some(&mut state.vs_constants));
            ctx.IAGetIndexBuffer(
                Some(&mut state.index),
                Some(&mut state.index_format),
                Some(&mut state.index_offset),
            );
            ctx.IAGetVertexBuffers(
                0,
                1,
                Some(&mut state.vertex),
                Some(&mut state.vertex_stride),
                Some(&mut state.vertex_offset),
            );
            ctx.OMGetRenderTargets(Some(&mut state.targets), Some(&mut state.depth_view));
            state
        }
    }

    fn restore(self, ctx: &ID3D11DeviceContext) {
        unsafe {
            ctx.RSSetScissorRects(Some(&self.scissors));
            ctx.RSSetViewports(Some(&self.viewports));
            ctx.RSSetState(self.raster.as_ref());
            ctx.OMSetBlendState(self.blend.as_ref(), Some(&self.blend_factor), self.sample_mask);
            ctx.OMSetDepthStencilState(self.depth.as_ref(), self.stencil_ref);
            ctx.PSSetShaderResources(0, Some(&self.ps_resource));
            ctx.PSSetSamplers(0, Some(&self.ps_sampler));
            ctx.PSSetShader(self.ps.as_ref(), None);
            ctx.VSSetShader(self.vs.as_ref(), None);
            ctx.GSSetShader(self.gs.as_ref(), None);
            ctx.HSSetShader(self.hs.as_ref(), None);
            ctx.DSSetShader(self.ds.as_ref(), None);
            ctx.CSSetShader(self.cs.as_ref(), None);
            ctx.VSSetConstantBuffers(0, Some(&self.vs_constants));
            ctx.IASetPrimitiveTopology(self.topology);
            ctx.IASetIndexBuffer(self.index.as_ref(), self.index_format, self.index_offset);
            ctx.IASetVertexBuffers(
                0,
                1,
                Some(&self.vertex),
                Some(&self.vertex_stride),
                Some(&self.vertex_offset),
            );
            ctx.IASetInputLayout(self.layout.as_ref());
            let count = self.targets.iter().take_while(|t| t.is_some()).count();
            ctx.OMSetRenderTargets(Some(&self.targets[..count]), self.depth_view.as_ref());
        }
    }
}

/// Called from the `Present` hook.
pub fn frame(this: *mut c_void) {
    if FAILED.load(Ordering::Relaxed) {
        return;
    }
    let mut guard = RENDERER.lock();
    if guard.as_ref().is_some_and(|r| r.swap != this as usize) {
        *guard = None;
    }
    if guard.is_none() {
        match Renderer::new(this) {
            Ok(renderer) => {
                tracing::info!("overlay renderer ready");
                *guard = Some(renderer);
            }
            Err(error) => {
                tracing::warn!(%error, "overlay renderer failed; in-game interface disabled");
                FAILED.store(true, Ordering::Relaxed);
                return;
            }
        }
    }
    if let Some(renderer) = guard.as_mut()
        && let Err(error) = renderer.draw()
    {
        tracing::warn!(%error, "overlay draw failed; in-game interface disabled");
        FAILED.store(true, Ordering::Relaxed);
        *guard = None;
    }
}

/// Drops the back-buffer view before `ResizeBuffers`.
pub fn before_resize() {
    if let Some(renderer) = RENDERER.lock().as_mut() {
        renderer.target = None;
    }
}

pub fn release() {
    *RENDERER.lock() = None;
}
