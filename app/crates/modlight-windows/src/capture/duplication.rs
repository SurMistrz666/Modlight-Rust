//! The screen through DXGI Desktop Duplication: what the recorder reads first, and what
//! a screenshot falls back on when GDI shows a monitor black.
//!
//! GDI cannot see a flip-model swap chain presented with independent flip, which is how
//! DX12, Vulkan and many DX11 games run in fullscreen and borderless fullscreen: `BitBlt`
//! gives black there. Duplication hands over what the monitor shows.
//!
//! It works per output, each on a D3D11 device of that output's own adapter (a device on
//! another adapter makes `DuplicateOutput` fail on hybrid-GPU laptops), so a region is
//! split into one part per output ([`split`]). A part goes through GDI whenever its
//! duplication cannot serve it: not opened, lost, rotated, in another format, or still
//! without a picture ([`Picture`]). So GDI is the whole recording when duplication cannot
//! start at all (a remote session, no hardware) and one part of it while an output
//! recovers. No failure here stops a recording.
//!
//! DXGI desktop coordinates are physical pixels, the space of `ScreenFrames`: the caller
//! holds [`PhysicalPixels`](super::desktop::PhysicalPixels). Every object here lives and
//! dies on the thread that made it.

use std::time::Instant;

use modlight_core::capture::geometry::IRect;
use winrt::Win32::Foundation::{E_INVALIDARG, E_POINTER, HMODULE};
use winrt::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use winrt::Win32::Graphics::Direct3D11::{
    D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use winrt::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_MODE_ROTATION, DXGI_MODE_ROTATION_IDENTITY,
    DXGI_MODE_ROTATION_UNSPECIFIED, DXGI_SAMPLE_DESC,
};
use winrt::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ERROR_DEVICE_HUNG, DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET,
    DXGI_ERROR_NOT_FOUND, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO, IDXGIAdapter1,
    IDXGIFactory1, IDXGIOutput, IDXGIOutput1, IDXGIOutput5, IDXGIOutputDuplication, IDXGIResource,
};
use winrt::core::{Error, HRESULT, Interface};

use super::desktop::Monitor;
use super::gdi::{Dib, ScreenDc};
use crate::duplication_plan::{
    Path, PathLog, Picture, Piece, Retry, copy_rows, is_black, leaves_the_desktop, mapped_length,
    path_of, reopen_at, replan_at, split, wait_for_picture,
};
use crate::wide::from_wide;

/// An adapter by its LUID, which outlives the objects of one enumeration.
type AdapterId = (u32, i32);

/// Why a duplication is given up for now.
struct Problem {
    /// For the log.
    text: String,
    /// The result of the call that failed; `None` when the calls were fine and the
    /// picture cannot be read as it is.
    code: Option<HRESULT>,
}

impl Problem {
    fn call(what: &str, error: &Error) -> Problem {
        Problem::code(what, error.code())
    }

    fn code(what: &str, code: HRESULT) -> Problem {
        Problem {
            text: format!("{what} failed with {}", hex(code)),
            code: Some(code),
        }
    }

    fn unsuitable(text: String) -> Problem {
        Problem { text, code: None }
    }
}

/// An HRESULT as Microsoft's pages write it.
fn hex(code: HRESULT) -> String {
    format!("{:#010X}", code.0 as u32)
}

/// One output on the desktop, as one enumeration found it.
struct Output {
    adapter: IDXGIAdapter1,
    adapter_id: AdapterId,
    output: IDXGIOutput,
    /// The GDI device name (`\\.\DISPLAY1`), for the log.
    name: String,
    /// Physical pixels.
    bounds: IRect,
    /// Its picture has the desktop's orientation.
    upright: bool,
}

/// The rotations that need no turning: identity, and "not said".
fn upright(rotation: DXGI_MODE_ROTATION) -> bool {
    rotation == DXGI_MODE_ROTATION_IDENTITY || rotation == DXGI_MODE_ROTATION_UNSPECIFIED
}

/// Every output attached to the desktop, on every adapter. A new factory each time: one
/// made earlier keeps the adapters and outputs of back then.
fn find_outputs() -> Result<Vec<Output>, Problem> {
    // SAFETY: a plain call that returns a new object.
    let factory: IDXGIFactory1 =
        unsafe { CreateDXGIFactory1() }.map_err(|e| Problem::call("CreateDXGIFactory1", &e))?;
    let mut found = Vec::new();
    for index in 0u32.. {
        // SAFETY: a live factory; past the last adapter it answers DXGI_ERROR_NOT_FOUND.
        let adapter = match unsafe { factory.EnumAdapters1(index) } {
            Ok(adapter) => adapter,
            Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(error) => return Err(Problem::call("EnumAdapters1", &error)),
        };
        // SAFETY: a live adapter. One that cannot describe itself is left out: GDI reads
        // its monitors.
        let Ok(description) = (unsafe { adapter.GetDesc1() }) else {
            continue;
        };
        let luid = description.AdapterLuid;
        add_outputs(&adapter, (luid.LowPart, luid.HighPart), &mut found);
    }
    Ok(found)
}

fn add_outputs(adapter: &IDXGIAdapter1, adapter_id: AdapterId, found: &mut Vec<Output>) {
    for index in 0u32.. {
        // SAFETY: a live adapter; past the last output it answers DXGI_ERROR_NOT_FOUND, and
        // any other answer ends its list as well.
        let Ok(output) = (unsafe { adapter.EnumOutputs(index) }) else {
            break;
        };
        // SAFETY: a live output. One that cannot say where it is stays with GDI.
        let Ok(description) = (unsafe { output.GetDesc() }) else {
            continue;
        };
        if !description.AttachedToDesktop.as_bool() {
            continue;
        }
        let rect = description.DesktopCoordinates;
        found.push(Output {
            adapter: adapter.clone(),
            adapter_id,
            output,
            name: from_wide(&description.DeviceName),
            bounds: IRect::new(
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
            ),
            upright: upright(description.Rotation),
        });
    }
}

/// A D3D11 device of one adapter.
#[derive(Clone)]
struct Device {
    adapter_id: AdapterId,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
}

impl Device {
    fn new(output: &Output) -> Result<Device, Problem> {
        let (mut device, mut context) = (None, None);
        // SAFETY: a live adapter with the driver type that goes with naming one, no
        // software module, and out-parameters that outlive the call.
        unsafe {
            D3D11CreateDevice(
                &output.adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                // The staging textures are B8G8R8A8.
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        }
        .map_err(|e| Problem::call("D3D11CreateDevice", &e))?;
        match (device, context) {
            (Some(device), Some(context)) => Ok(Device {
                adapter_id: output.adapter_id,
                device,
                context,
            }),
            _ => Err(Problem::code("D3D11CreateDevice", E_POINTER)),
        }
    }
}

/// The devices in use: one per adapter, shared by that adapter's outputs.
#[derive(Default)]
struct Devices(Vec<Device>);

impl Devices {
    fn of(&mut self, output: &Output) -> Result<Device, Problem> {
        let known = self.0.iter().find(|d| d.adapter_id == output.adapter_id);
        if let Some(device) = known {
            return Ok(device.clone());
        }
        let device = Device::new(output)?;
        self.0.push(device.clone());
        Ok(device)
    }

    /// Lets go of this device: the next output of its adapter gets a new one. Outputs that
    /// use it keep it until they fail by themselves, as they do when it is lost.
    fn forget(&mut self, device: &ID3D11Device) {
        self.0
            .retain(|known| known.device.as_raw() != device.as_raw());
    }
}

/// `IDXGIOutput5::DuplicateOutput1` asking for B8G8R8A8 (so an HDR desktop comes in that
/// format), else `IDXGIOutput1::DuplicateOutput` on the same output: on a Windows without
/// the first (10 before 1703), and after the first failed, whatever the error.
/// `DuplicateOutput1` is reported to answer DXGI_ERROR_UNSUPPORTED to a process that is not
/// per-monitor DPI aware; Modlight.exe is one, a test binary only has the capture thread's
/// own awareness. With the duplication, which call made it, for the log.
fn duplicate(
    output: &IDXGIOutput,
    device: &ID3D11Device,
) -> Result<(IDXGIOutputDuplication, String), Problem> {
    let mut first_error = None;
    if let Ok(output5) = output.cast::<IDXGIOutput5>() {
        let formats = [DXGI_FORMAT_B8G8R8A8_UNORM];
        // SAFETY: a live output and a live device of its adapter; the formats are read
        // during the call.
        match unsafe { output5.DuplicateOutput1(device, 0, &formats) } {
            Ok(duplication) => return Ok((duplication, "DuplicateOutput1".into())),
            Err(error) => first_error = Some(error.code()),
        }
    }
    let output1: IDXGIOutput1 = output
        .cast()
        .map_err(|e| Problem::call("IDXGIOutput1", &e))?;
    // SAFETY: as above.
    match unsafe { output1.DuplicateOutput(device) } {
        Ok(duplication) => {
            let call = match first_error {
                Some(code) => format!("DuplicateOutput (DuplicateOutput1 said {})", hex(code)),
                None => "DuplicateOutput".to_string(),
            };
            Ok((duplication, call))
        }
        Err(error) => {
            let mut problem = Problem::call("DuplicateOutput", &error);
            if let Some(code) = first_error {
                problem.text += &format!(", DuplicateOutput1 with {}", hex(code));
            }
            Err(problem)
        }
    }
}

/// The texture a part's picture is kept in between frames, readable by the CPU.
fn staging_texture(device: &ID3D11Device, piece: &Piece) -> Result<ID3D11Texture2D, Problem> {
    let description = D3D11_TEXTURE2D_DESC {
        Width: piece.source.width as u32,
        Height: piece.source.height as u32,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };
    let mut texture = None;
    // SAFETY: a live device, a description that outlives the call, no initial data (a
    // staging texture needs none), and an out-parameter.
    unsafe { device.CreateTexture2D(&description, None, Some(&mut texture)) }
        .map_err(|e| Problem::call("CreateTexture2D", &e))?;
    texture.ok_or_else(|| Problem::code("CreateTexture2D", E_POINTER))
}

/// Copies a part of an acquired picture into the staging texture, after checking that the
/// picture is what the part expects. The format is read from the texture itself, the one
/// thing sure to describe these pixels: `DXGI_OUTDUPL_DESC` describes the desktop, and on
/// an HDR display that may not be the format `DuplicateOutput1` was asked to deliver. A copy
/// between two formats or outside the picture would fail without a word, and leave the
/// old picture or none.
fn copy_picture(
    context: &ID3D11DeviceContext,
    staging: &ID3D11Texture2D,
    piece: &Piece,
    size: (u32, u32),
    resource: &IDXGIResource,
) -> Result<(), Problem> {
    let texture: ID3D11Texture2D = resource
        .cast()
        .map_err(|e| Problem::call("ID3D11Texture2D", &e))?;
    let mut description = D3D11_TEXTURE2D_DESC::default();
    // SAFETY: a live texture and a structure to fill.
    unsafe { texture.GetDesc(&mut description) };
    if description.Format != DXGI_FORMAT_B8G8R8A8_UNORM {
        return Err(Problem::unsuitable(format!(
            "its picture has format {}, not B8G8R8A8",
            description.Format.0
        )));
    }
    if (description.Width, description.Height) != size {
        return Err(Problem::unsuitable(format!(
            "its picture is {} x {}, the output {} x {}",
            description.Width, description.Height, size.0, size.1
        )));
    }
    let source = piece.source;
    let area = D3D11_BOX {
        left: source.x as u32,
        top: source.y as u32,
        front: 0,
        right: source.right() as u32,
        bottom: source.bottom() as u32,
        back: 1,
    };
    // SAFETY: two live textures of one device and one format; the box lies inside the
    // source (`piece` is inside the output, whose size was just compared) and has the
    // staging texture's size. The box outlives the call.
    unsafe { context.CopySubresourceRegion(staging, 0, 0, 0, 0, &texture, 0, Some(&area)) };
    Ok(())
}

/// A frame from `AcquireNextFrame` and its picture. Both go back on every way out, the
/// picture first: by [`release`](HeldFrame::release), or when dropped on an error path.
struct HeldFrame<'a> {
    duplication: &'a IDXGIOutputDuplication,
    resource: Option<IDXGIResource>,
    held: bool,
}

impl HeldFrame<'_> {
    fn release(mut self) -> Result<(), Problem> {
        self.held = false;
        self.resource = None;
        // SAFETY: the frame this value holds, given back once, nothing of it kept.
        unsafe { self.duplication.ReleaseFrame() }.map_err(|e| Problem::call("ReleaseFrame", &e))
    }
}

impl Drop for HeldFrame<'_> {
    fn drop(&mut self) {
        if self.held {
            self.resource = None;
            // SAFETY: as `release`. Only an error path comes here, and its error is the
            // one that counts.
            let _ = unsafe { self.duplication.ReleaseFrame() };
        }
    }
}

/// A staging texture mapped for reading, unmapped when dropped, on every path.
struct Mapped<'a> {
    context: &'a ID3D11DeviceContext,
    texture: &'a ID3D11Texture2D,
    data: D3D11_MAPPED_SUBRESOURCE,
}

impl<'a> Mapped<'a> {
    fn new(
        context: &'a ID3D11DeviceContext,
        texture: &'a ID3D11Texture2D,
    ) -> Result<Mapped<'a>, Problem> {
        let mut data = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: a texture made with CPU read access, its only subresource, and an
        // out-parameter. This waits for the copy queued into it. A failed Map maps
        // nothing, so there is nothing to unmap then.
        unsafe { context.Map(texture, 0, D3D11_MAP_READ, 0, Some(&mut data)) }
            .map_err(|e| Problem::call("Map", &e))?;
        Ok(Mapped {
            context,
            texture,
            data,
        })
    }

    /// The picture's bytes: `length` of them, rows `RowPitch` apart.
    fn bytes(&self, length: usize) -> &[u8] {
        // SAFETY: while mapped, `pData` is the texture's memory: `RowPitch` bytes for each
        // of its rows but the last, which has at least a row of pixels. The caller's
        // `length` comes from `mapped_length` and is no more than that. The slice cannot
        // outlive `self`, which unmaps.
        unsafe { std::slice::from_raw_parts(self.data.pData as *const u8, length) }
    }
}

impl Drop for Mapped<'_> {
    fn drop(&mut self) {
        // SAFETY: the subresource `new` mapped, once.
        unsafe { self.context.Unmap(self.texture, 0) };
    }
}

/// One output's duplication and what it needs to serve one part.
struct Open {
    /// First of the objects: it goes before the device it was made on.
    duplication: IDXGIOutputDuplication,
    staging: ID3D11Texture2D,
    device: Device,
    piece: Piece,
    /// The output's size: what an acquired picture must have.
    size: (u32, u32),
    /// Which call made the duplication.
    call: String,
    picture: Picture,
}

impl Open {
    fn new(output: &Output, piece: Piece, devices: &mut Devices) -> Result<Open, Problem> {
        // Known before anything is opened: a portrait monitor stays one for the whole
        // recording, and is not to be opened and closed again and again.
        if !output.upright {
            return Err(Problem::unsuitable("the output is rotated".into()));
        }
        let device = devices.of(output)?;
        let (duplication, call) =
            duplicate(&output.output, &device.device).inspect_err(|problem| {
                // By the documentation, among others: a device of another adapter, or this
                // process duplicating the output already. The next attempt gets a new one.
                if problem.code == Some(E_INVALIDARG) {
                    devices.forget(&device.device);
                }
            })?;
        // SAFETY: a live duplication; the call fills and returns a structure.
        let description = unsafe { duplication.GetDesc() };
        if !upright(description.Rotation) {
            return Err(Problem::unsuitable("the output is rotated".into()));
        }
        let size = (output.bounds.width as u32, output.bounds.height as u32);
        let mode = (description.ModeDesc.Width, description.ModeDesc.Height);
        if mode != size {
            return Err(Problem::unsuitable(format!(
                "its mode is {} x {}, the output {} x {}",
                mode.0, mode.1, size.0, size.1
            )));
        }
        let staging = staging_texture(&device.device, &piece)?;
        Ok(Open {
            duplication,
            staging,
            device,
            piece,
            size,
            call,
            picture: Picture::default(),
        })
    }

    /// One `AcquireNextFrame`, waiting at most `timeout` milliseconds. A new picture goes
    /// to the staging texture and the frame goes back at once; a timeout means nothing
    /// changed, and the staging texture still holds the last picture.
    fn take(&mut self, timeout: u32) -> Result<(), Problem> {
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;
        // SAFETY: a live duplication with no frame held (every frame taken here goes back
        // before this returns), and out-parameters that outlive the call.
        let acquired = unsafe {
            self.duplication
                .AcquireNextFrame(timeout, &mut info, &mut resource)
        };
        match acquired {
            Ok(()) => {}
            Err(error) if error.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(()),
            Err(error) => return Err(Problem::call("AcquireNextFrame", &error)),
        }
        let frame = HeldFrame {
            duplication: &self.duplication,
            resource,
            held: true,
        };
        if let Some(resource) = &frame.resource {
            let (context, staging) = (&self.device.context, &self.staging);
            let (piece, size) = (&self.piece, self.size);
            self.picture.offer(info.LastPresentTime, || {
                copy_picture(context, staging, piece, size, resource)
            })?;
        }
        frame.release()
    }

    /// Copies the last picture into its place in `dib`; `false` when there is none yet.
    fn draw(&self, dib: &mut Dib) -> Result<bool, Problem> {
        if !self.picture.held() {
            return Ok(false);
        }
        let mapped = Mapped::new(&self.device.context, &self.staging)?;
        let pitch = mapped.data.RowPitch as usize;
        let dest = self.piece.dest;
        let length = mapped_length(pitch, dest.width as usize, dest.height as usize)
            .filter(|_| !mapped.data.pData.is_null())
            .ok_or_else(|| Problem::code("Map", E_POINTER))?;
        let width = dib.width as usize;
        if !copy_rows(mapped.bytes(length), pitch, dib.bgra_mut(), width, dest) {
            return Err(Problem::unsuitable("its picture does not fit".into()));
        }
        Ok(true)
    }

    /// This failure means the device is gone, not just this output's duplication.
    fn device_lost(&self, problem: &Problem) -> bool {
        let lost = [
            DXGI_ERROR_DEVICE_REMOVED,
            DXGI_ERROR_DEVICE_RESET,
            DXGI_ERROR_DEVICE_HUNG,
        ];
        // SAFETY: a plain query on a live device.
        problem.code.is_some_and(|code| lost.contains(&code))
            || unsafe { self.device.device.GetDeviceRemovedReason() }.is_err()
    }
}

/// Where an output's duplication stands.
enum State {
    /// Its part comes from GDI; it may be opened from this moment on.
    Closed {
        not_before: Instant,
    },
    Open(Open),
}

/// One part of the region and how it is read.
struct Slot {
    piece: Piece,
    /// The output, for the log.
    name: String,
    state: State,
    /// The call last logged for this part, and the problem: each is said when it changes.
    call: Option<String>,
    problem: Option<String>,
}

impl Slot {
    fn new(piece: Piece, name: String, now: Instant) -> Slot {
        Slot {
            piece,
            name,
            state: State::Closed { not_before: now },
            call: None,
            problem: None,
        }
    }

    fn due(&self, now: Instant) -> bool {
        matches!(self.state, State::Closed { not_before } if now >= not_before)
    }

    fn has_picture(&self) -> bool {
        matches!(&self.state, State::Open(open) if open.picture.held())
    }

    fn waits_for_picture(&self) -> bool {
        matches!(&self.state, State::Open(open) if !open.picture.held())
    }

    fn open(&mut self, output: &Output, devices: &mut Devices, now: Instant) {
        match Open::new(output, self.piece, devices) {
            Ok(open) => {
                if self.call.as_ref() != Some(&open.call) {
                    log::info!(
                        "Desktop Duplication of {} opened with {}; GDI reads it until its first picture",
                        self.name,
                        open.call
                    );
                    self.call = Some(open.call.clone());
                }
                self.state = State::Open(open);
            }
            Err(problem) => self.give_up(problem, now),
        }
    }

    /// Reads this part into `dib`; `false` leaves it to GDI for this frame.
    fn draw(&mut self, dib: &mut Dib, devices: &mut Devices) -> bool {
        let State::Open(open) = &mut self.state else {
            return false;
        };
        let drawn = open.take(0).and_then(|()| open.draw(dib));
        match drawn {
            Ok(drawn) => drawn,
            Err(problem) => {
                if open.device_lost(&problem) {
                    devices.forget(&open.device.device);
                }
                self.give_up(problem, Instant::now());
                false
            }
        }
    }

    /// Drops the duplication (replacing the state lets go of it) and says why, once.
    fn give_up(&mut self, problem: Problem, now: Instant) {
        self.state = State::Closed {
            not_before: reopen_at(now, problem.code.is_none()),
        };
        if self.problem.as_ref() != Some(&problem.text) {
            log::warn!(
                "Desktop Duplication of {}: {}; GDI reads it meanwhile",
                self.name,
                problem.text
            );
            self.problem = Some(problem.text);
        }
    }
}

/// The recorder's reading of one region, on the capture thread: duplication where it
/// serves, GDI everywhere else.
pub(crate) struct Duplication {
    region: IRect,
    /// The outputs the parts were planned on, as the last enumeration found them.
    screens: Vec<IRect>,
    slots: Vec<Slot>,
    /// What no output shows; GDI reads it, and off the monitors it stays black.
    rest: Vec<IRect>,
    devices: Devices,
    retry: Retry,
    /// When the outputs are to be enumerated again though no part asks for it: before
    /// the first plan, after a change of the monitors, and now and then while no output
    /// shows the region.
    replan: Option<Instant>,
    path: PathLog,
    /// The last problem finding the outputs, said when it changes.
    problem: Option<String>,
}

impl Duplication {
    /// Nothing is opened yet: until [`recover`](Duplication::recover) has run, this is
    /// plain GDI, and it stays that when no output can be found.
    pub(crate) fn new(region: IRect) -> Duplication {
        Duplication {
            region,
            screens: Vec::new(),
            slots: Vec::new(),
            rest: vec![region],
            devices: Devices::default(),
            retry: Retry::default(),
            replan: Some(Instant::now()),
            path: PathLog::default(),
            problem: None,
        }
    }

    /// The monitors changed: everything is planned again, at the next `recover`.
    pub(crate) fn rebuild(&mut self) {
        self.screens.clear();
        self.slots.clear();
        self.rest = vec![self.region];
        self.devices = Devices::default();
        self.retry = Retry::default();
        self.replan = Some(Instant::now());
    }

    /// One frame's pixels into `dib`, all of them fresh, so the pointer drawn into the
    /// last frame never stays behind. `bounds` is the virtual desktop.
    pub(crate) fn draw(&mut self, dib: &mut Dib, screen: &ScreenDc, bounds: IRect) {
        let pieces = self.slots.iter().map(|slot| &slot.piece.desktop);
        if leaves_the_desktop(self.rest.iter().chain(pieces), bounds) {
            // Off every monitor stays black.
            dib.fill([0, 0, 0, 255]);
        }
        for slot in &mut self.slots {
            if !slot.draw(dib, &mut self.devices) {
                read_with_gdi(dib, screen, self.region, slot.piece.desktop, bounds);
            }
        }
        for part in &self.rest {
            read_with_gdi(dib, screen, self.region, *part, bounds);
        }
    }

    /// Opens what is closed, at most every 500 ms, and says what the recording uses when
    /// that changed. For after a frame is out: opening can take a while, and a slow driver
    /// must not hold up the frame (the first one is on the clock).
    pub(crate) fn recover(&mut self) {
        let now = Instant::now();
        let replan = self.replan.is_some_and(|at| now >= at);
        let wanted = replan || self.slots.iter().any(|slot| slot.due(now));
        if wanted && self.retry.due(now) {
            self.attempt(now);
        }
        self.report();
    }

    fn attempt(&mut self, now: Instant) {
        let outputs = match find_outputs() {
            Ok(outputs) => outputs,
            Err(problem) => {
                if self.problem.as_ref() != Some(&problem.text) {
                    log::warn!("Desktop Duplication: {}", problem.text);
                    self.problem = Some(problem.text);
                }
                return;
            }
        };
        let screens: Vec<IRect> = outputs.iter().map(|output| output.bounds).collect();
        if screens != self.screens {
            // Other outputs, or the same in other places: the parts are new.
            let plan = split(self.region, &screens);
            self.slots = plan
                .pieces
                .iter()
                .map(|piece| Slot::new(*piece, outputs[piece.output].name.clone(), now))
                .collect();
            self.rest = plan.rest;
            self.screens = screens;
        }
        self.replan = replan_at(now, self.slots.len());
        // The same outputs in the same order: each part's output is where it was.
        for slot in &mut self.slots {
            if slot.due(now) {
                slot.open(&outputs[slot.piece.output], &mut self.devices, now);
            }
        }
    }

    /// How many parts come from duplication now.
    pub(crate) fn pictures(&self) -> usize {
        self.slots.iter().filter(|slot| slot.has_picture()).count()
    }

    /// Says the path once it is known: a duplication just opened owes its first picture,
    /// and its "opened" line says GDI reads that part until then.
    fn report(&mut self) {
        if self.slots.iter().any(Slot::waits_for_picture) {
            return;
        }
        let path = path_of(self.pictures(), self.slots.len());
        match self.path.update(path) {
            Some(Path::Duplication) => {
                log::info!("the recording reads the screen through Desktop Duplication");
            }
            Some(Path::Mixed { duplicated, parts }) => log::warn!(
                "the recording reads {duplicated} of {parts} monitors through Desktop Duplication, the rest through GDI"
            ),
            Some(Path::Gdi) => log::warn!("the recording reads the screen through GDI"),
            None => {}
        }
    }

    /// What each part reads from and why, for a test run by hand to print.
    #[cfg(test)]
    pub(crate) fn describe(&self) -> String {
        let mut lines: Vec<String> = self.problem.iter().cloned().collect();
        for slot in &self.slots {
            let state = match &slot.state {
                State::Open(open) if open.picture.held() => "a picture",
                State::Open(_) => "open, no picture yet",
                State::Closed { .. } => "closed",
            };
            lines.push(format!(
                "{}: {state}; opened with {:?}; last problem {:?}",
                slot.name, slot.call, slot.problem
            ));
        }
        lines.join("\n")
    }
}

/// The part of `part` on the desktop, read with GDI into its place in the frame of
/// `region`.
fn read_with_gdi(dib: &Dib, screen: &ScreenDc, region: IRect, part: IRect, bounds: IRect) {
    let part = part.intersect(&bounds);
    if part.has_area() {
        dib.blit_screen(screen, part, part.x - region.x, part.y - region.y);
    }
}

/// One picture of one part, if a new one comes in time.
fn take_once(
    output: &Output,
    piece: Piece,
    devices: &mut Devices,
    dib: &mut Dib,
) -> Result<Option<String>, Problem> {
    let mut open = Open::new(output, piece, devices)?;
    let taken = wait_for_picture(Instant::now, |left| {
        open.take(left)?;
        Ok(open.picture.held())
    })?;
    Ok((taken && open.draw(dib)?).then_some(open.call))
}

/// A screenshot's second look: `dib` holds `region` as GDI saw it, and each monitor's part
/// that came out entirely black is taken once more through duplication, in case a game
/// that GDI cannot see is there. A failure of any kind keeps the GDI picture. A desktop
/// with nothing black on it makes no DXGI call at all.
pub(crate) fn retake_black_parts(dib: &mut Dib, region: IRect, monitors: &[Monitor]) {
    let width = dib.width as usize;
    let screens: Vec<IRect> = monitors.iter().map(|monitor| monitor.bounds).collect();
    let black = |dib: &Dib, piece: &Piece| is_black(dib.bgra(), width, piece.dest);
    if !split(region, &screens).pieces.iter().any(|p| black(dib, p)) {
        return;
    }
    let outputs = match find_outputs() {
        Ok(outputs) => outputs,
        Err(problem) => {
            log::debug!("a black capture stays as it is: {}", problem.text);
            return;
        }
    };
    let bounds: Vec<IRect> = outputs.iter().map(|output| output.bounds).collect();
    let mut devices = Devices::default();
    for piece in split(region, &bounds).pieces {
        if !black(dib, &piece) {
            continue;
        }
        let output = &outputs[piece.output];
        match take_once(output, piece, &mut devices, dib) {
            Ok(Some(call)) => log::info!(
                "a part of a capture that GDI saw black came from Desktop Duplication ({call})"
            ),
            Ok(None) => log::debug!("no new picture of {} in time", output.name),
            Err(problem) => log::debug!("{}: {}", output.name, problem.text),
        }
    }
}
