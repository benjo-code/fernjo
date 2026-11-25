//! Caps on what a server's display updates can make the client hold.
//!
//! Two structures of `ironrdp-session` 0.13.0 (`fast_path.rs`) grow from server input without a
//! bound of their own:
//!
//! * `CompleteData` reassembles a fragmented fast-path update (`Fragmentation::First`, then `Next`
//!   until `Last`) by appending every fragment to one buffer, and nothing compares the total with
//!   the `MultifragmentUpdate` size the client confirmed. [`UpdateCheck`] follows IronRDP's rules
//!   for that buffer (`First` and `Single` drop a pending update, `Next` and `Last` append, `Last`
//!   hands it on) and refuses the fragment that takes the pending update above [`fragment_cap`].
//! * `PointerCache` is a `HashMap` that takes any `u16` cache index, and every pointer is decoded
//!   at width x height x 4 bytes whatever its wire size. A pointer update that carries a cache
//!   index or a size (color, new, large and cached pointer; fast path and slow path, which share
//!   IronRDP's `process_pointer_update`) is refused when the index is at or above
//!   [`POINTER_CACHE_SIZE`] or a side is above [`MAX_POINTER_SIDE`]. The first fragment of a
//!   fragmented pointer update must hold the whole pointer header, so a header split over
//!   fragments cannot hide its index or size.
//!
//! Like the channel cap, the check runs before the frame reaches the active stage, and anything
//! it cannot decode passes: IronRDP then ends the session with its own error or discards the
//! update, and the check follows that.

use ironrdp_core::{ReadCursor, decode_cursor};
use ironrdp_pdu::Action;
use ironrdp_pdu::fast_path::{FastPathHeader, FastPathUpdatePdu, Fragmentation, UpdateCode};
use ironrdp_pdu::mcs::SendDataIndicationCtx;
use ironrdp_pdu::rdp::headers::{IoChannelPdu, ShareDataPdu, decode_io_channel};

use crate::error::UpdateOutOfLimits;

/// The pointer cache size the client advertises: `DEFAULT_POINTER_CACHE_SIZE` of
/// `ironrdp-connector` 0.12.0, sent as both `color_pointer_cache_size` and `pointer_cache_size`.
/// MS-RDPBCGR 2.2.9.1.1.4.4: the cache index is a zero-based entry of that cache.
pub(crate) const POINTER_CACHE_SIZE: u16 = 32;

/// The largest pointer side the client advertises (`LargePointerSupportFlags::UP_TO_384X384_PIXELS`,
/// MS-RDPBCGR 2.2.9.1.2.1.11). The spec allows only 96 for color and new pointers, but 384 is
/// applied to every kind so a lax server is not cut off; 32 cached pointers of 384 x 384 pixels
/// hold at most 18 MiB.
pub(crate) const MAX_POINTER_SIDE: u16 = 384;

/// The smallest cap on a fragmented fast-path update: the `MultifragmentUpdate` size IronRDP
/// confirms when the server proposes none (8 MiB).
pub(crate) const MIN_FRAGMENTED_UPDATE: u64 = 8 * 1024 * 1024;

/// The cap on one fragmented fast-path update for a desktop of this size: twice a raw 32-bit frame
/// of the desktop, at least [`MIN_FRAGMENTED_UPDATE`]. IronRDP echoes the size a server proposes,
/// so the confirmed size is the server's own choice and no bound; this cap is fernjo's. A server
/// that proposes a size sizes it for one whole screen update (FreeRDP's server: 16 KiB per 64 x 64
/// tile plus 16 KiB, which twice the raw frame covers for every desktop of at least 200 x 200), and
/// the cap is as large as the two copies of the desktop the session already holds (the decoded
/// image and the framebuffer).
pub(crate) fn fragment_cap(width: u16, height: u16) -> u64 {
    (8 * u64::from(width) * u64::from(height)).max(MIN_FRAGMENTED_UPDATE)
}

/// The fragmented fast-path update IronRDP is reassembling.
#[derive(Debug, Clone, Copy)]
struct Pending {
    code: UpdateCode,
    compressed: bool,
    len: u64,
}

/// The state of the display update pre-check for one activation of a session.
#[derive(Debug)]
pub(crate) struct UpdateCheck {
    io_channel: u16,
    cap: u64,
    pending: Option<Pending>,
}

impl UpdateCheck {
    pub(crate) fn new(io_channel: u16, width: u16, height: u16) -> Self {
        Self {
            io_channel,
            cap: fragment_cap(width, height),
            pending: None,
        }
    }

    /// After a deactivation-reactivation: IronRDP builds a new fast-path processor, so nothing is
    /// being reassembled any more, and the desktop may have a new size.
    pub(crate) fn reactivated(&mut self, io_channel: u16, width: u16, height: u16) {
        *self = Self::new(io_channel, width, height);
    }

    /// Refuses a frame with a display update beyond the limits; see the module documentation.
    pub(crate) fn check(
        &mut self,
        action: Action,
        payload: &[u8],
    ) -> Result<(), UpdateOutOfLimits> {
        match action {
            Action::FastPath => self.fast_path(payload),
            Action::X224 => self.slow_path(payload),
        }
    }

    /// Mirrors `fast_path::Processor::process` and `CompleteData::process_data`.
    fn fast_path(&mut self, payload: &[u8]) -> Result<(), UpdateOutOfLimits> {
        let mut input = ReadCursor::new(payload);
        if decode_cursor::<FastPathHeader>(&mut input).is_err() {
            // IronRDP refuses the frame and the session ends
            return Ok(());
        }
        while !input.is_empty() {
            let Ok(update) = decode_cursor::<FastPathUpdatePdu<'_>>(&mut input) else {
                // IronRDP either discards the pending update together with the rest of the frame
                // (a truncated visual update) or ends the session
                self.pending = None;
                return Ok(());
            };
            if update
                .compression_flags
                .is_some_and(|flags| !flags.is_empty())
            {
                // fernjo negotiates no compression, so IronRDP has no decompressor and ends the
                // session on this update
                return Ok(());
            }
            let compressed = update.compression_flags.is_some();
            let len = u64::try_from(update.data.len()).unwrap_or(u64::MAX);
            match update.fragmentation {
                Fragmentation::Single => {
                    self.pending = None;
                    check_pointer(fast_path_pointer(update.update_code), update.data, true)?;
                }
                Fragmentation::First => {
                    check_pointer(fast_path_pointer(update.update_code), update.data, false)?;
                    self.pending = Some(Pending {
                        code: update.update_code,
                        compressed,
                        len,
                    });
                }
                Fragmentation::Next | Fragmentation::Last => {
                    let Some(pending) = self.pending.as_mut() else {
                        // a fragment without a sequence: IronRDP ends the session
                        return Ok(());
                    };
                    if pending.code != update.update_code || pending.compressed != compressed {
                        // inconsistent metadata: IronRDP ends the session
                        return Ok(());
                    }
                    pending.len = pending.len.saturating_add(len);
                    if pending.len > self.cap {
                        return Err(UpdateOutOfLimits);
                    }
                    if update.fragmentation == Fragmentation::Last {
                        self.pending = None;
                    }
                }
            }
        }
        Ok(())
    }

    /// Pointer updates on the I/O channel, walked like IronRDP's
    /// `x224::Processor::process_io_channel_data_indication` walks concatenated share control PDUs.
    fn slow_path(&self, payload: &[u8]) -> Result<(), UpdateOutOfLimits> {
        let Ok(data) = ironrdp_pdu::mcs::decode_send_data_indication(payload) else {
            return Ok(());
        };
        if data.channel_id != self.io_channel {
            return Ok(());
        }
        let user_data = data.user_data;
        // multitransport requests and redirections have a basic security header instead
        if user_data.get(2..4) == Some([0, 0].as_slice())
            && matches!(
                decode_io_channel(data),
                Ok(IoChannelPdu::MultitransportRequest(_) | IoChannelPdu::ServerRedirection(_))
            )
        {
            return Ok(());
        }
        let mut offset = 0usize;
        while offset < user_data.len() {
            let Some(length) = user_data.get(offset..offset + 2) else {
                // IronRDP: truncated share control length, the session ends
                return Ok(());
            };
            let total = usize::from(u16::from_le_bytes([length[0], length[1]]));
            if total == 0 || offset + total > user_data.len() {
                // IronRDP decodes the whole user data as one PDU when this is the first one, and
                // ends the session otherwise
                return if offset == 0 {
                    check_share_pdu(data)
                } else {
                    Ok(())
                };
            }
            check_share_pdu(SendDataIndicationCtx {
                initiator_id: data.initiator_id,
                channel_id: data.channel_id,
                user_data: &user_data[offset..offset + total],
            })?;
            offset += total;
        }
        Ok(())
    }
}

/// A slow-path pointer PDU (`TS_POINTER_PDU`, MS-RDPBCGR 2.2.9.1.1.4) inside one share control PDU.
fn check_share_pdu(ctx: SendDataIndicationCtx<'_>) -> Result<(), UpdateOutOfLimits> {
    let Ok(IoChannelPdu::Data(share)) = decode_io_channel(ctx) else {
        return Ok(());
    };
    let ShareDataPdu::Pointer(body) = share.pdu else {
        return Ok(());
    };
    if !share.compression_flags.is_empty() {
        // no decompressor (fernjo negotiates no compression): IronRDP ends the session
        return Ok(());
    }
    let Some(message_type) = body.get(0..2) else {
        return Ok(());
    };
    let kind = slow_path_pointer(u16::from_le_bytes([message_type[0], message_type[1]]));
    // messageType and pad come before the pointer attribute
    check_pointer(kind, body.get(4..).unwrap_or_default(), true)
}

/// The pointer updates that carry a cache index (and, except the cached pointer, a size).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PointerKind {
    /// `TS_COLORPOINTERATTRIBUTE`: cache index at 0, width at 6, height at 8.
    Color,
    /// `TS_CACHEDPOINTERATTRIBUTE`: cache index at 0.
    Cached,
    /// `TS_POINTERATTRIBUTE`: `xorBpp`, then a color pointer attribute.
    New,
    /// `TS_FP_LARGEPOINTERATTRIBUTE`: `xorBpp`, cache index at 2, width at 8, height at 10.
    Large,
}

fn fast_path_pointer(code: UpdateCode) -> Option<PointerKind> {
    match code {
        UpdateCode::ColorPointer => Some(PointerKind::Color),
        UpdateCode::CachedPointer => Some(PointerKind::Cached),
        UpdateCode::NewPointer => Some(PointerKind::New),
        UpdateCode::LargePointer => Some(PointerKind::Large),
        _ => None,
    }
}

/// The slow-path `messageType` values IronRDP's `decode_slow_path_pointer` maps to the same
/// attributes.
fn slow_path_pointer(message_type: u16) -> Option<PointerKind> {
    match message_type {
        0x0006 => Some(PointerKind::Color),
        0x0007 => Some(PointerKind::Cached),
        0x0008 => Some(PointerKind::New),
        0x0009 => Some(PointerKind::Large),
        _ => None,
    }
}

/// Refuses a pointer whose cache index or size is beyond the limits. `whole` says that `data` is
/// the complete update: a short one then passes (IronRDP refuses or ignores it), while a first
/// fragment too short for its header is refused, because the rest of the header would follow
/// unseen.
fn check_pointer(
    kind: Option<PointerKind>,
    data: &[u8],
    whole: bool,
) -> Result<(), UpdateOutOfLimits> {
    let Some(kind) = kind else {
        return Ok(());
    };
    let (index_at, size_at) = match kind {
        PointerKind::Color => (0, Some(6)),
        PointerKind::Cached => (0, None),
        PointerKind::New | PointerKind::Large => (2, Some(8)),
    };
    let header = size_at.map_or(index_at + 2, |at| at + 4);
    if data.len() < header {
        return if whole {
            Ok(())
        } else {
            Err(UpdateOutOfLimits)
        };
    }
    let field = |at: usize| u16::from_le_bytes([data[at], data[at + 1]]);
    if field(index_at) >= POINTER_CACHE_SIZE {
        return Err(UpdateOutOfLimits);
    }
    if let Some(at) = size_at
        && (field(at) > MAX_POINTER_SIDE || field(at + 2) > MAX_POINTER_SIDE)
    {
        return Err(UpdateOutOfLimits);
    }
    Ok(())
}

#[cfg(test)]
pub(super) mod tests {

    use ironrdp_core::encode_vec;
    use ironrdp_pdu::fast_path::EncryptionFlags;
    use ironrdp_pdu::pointer::{
        CachedPointerAttribute, ColorPointerAttribute, LargePointerAttribute, Point16,
        PointerAttribute,
    };
    use ironrdp_pdu::rdp::client_info::CompressionType;
    use ironrdp_pdu::rdp::headers::{
        CompressionFlags, ShareControlHeader, ShareControlPdu, ShareDataHeader,
    };

    use super::super::channel_cap::tests::indication;
    use super::*;

    const IO_CHANNEL: u16 = 1003;
    const FRAGMENT: usize = 16 * 1024;

    /// One fast-path update as the server encodes it.
    pub(in crate::session) fn update(
        code: UpdateCode,
        fragmentation: Fragmentation,
        data: &[u8],
    ) -> Vec<u8> {
        encode_vec(&FastPathUpdatePdu {
            fragmentation,
            update_code: code,
            compression_flags: None,
            compression_type: None,
            data,
        })
        .unwrap()
    }

    /// A fast-path frame holding these updates.
    pub(in crate::session) fn fast_path_frame(updates: &[Vec<u8>]) -> Vec<u8> {
        let body = updates.concat();
        let mut frame =
            encode_vec(&FastPathHeader::new(EncryptionFlags::empty(), body.len())).unwrap();
        frame.extend_from_slice(&body);
        frame
    }

    fn bitmap(fragmentation: Fragmentation) -> Vec<u8> {
        fast_path_frame(&[update(UpdateCode::Bitmap, fragmentation, &[0u8; FRAGMENT])])
    }

    fn check(check: &mut UpdateCheck, frame: &[u8]) -> Result<(), UpdateOutOfLimits> {
        check.check(Action::FastPath, frame)
    }

    fn color(cache_index: u16, width: u16, height: u16) -> ColorPointerAttribute<'static> {
        // empty masks are valid for the encoder and keep the frames small
        ColorPointerAttribute {
            cache_index,
            hot_spot: Point16 { x: 0, y: 0 },
            width,
            height,
            xor_mask: &[],
            and_mask: &[],
        }
    }

    /// The encoded pointer attribute of each kind with this index and size.
    pub(in crate::session) fn pointers(
        cache_index: u16,
        width: u16,
        height: u16,
    ) -> Vec<(UpdateCode, u16, Vec<u8>)> {
        let large = LargePointerAttribute {
            xor_bpp: 32,
            cache_index,
            hot_spot: Point16 { x: 0, y: 0 },
            width,
            height,
            xor_mask: &[],
            and_mask: &[],
        };
        let new = PointerAttribute {
            xor_bpp: 32,
            color_pointer: color(cache_index, width, height),
        };
        vec![
            (
                UpdateCode::ColorPointer,
                0x0006,
                encode_vec(&color(cache_index, width, height)).unwrap(),
            ),
            (
                UpdateCode::CachedPointer,
                0x0007,
                encode_vec(&CachedPointerAttribute { cache_index }).unwrap(),
            ),
            (UpdateCode::NewPointer, 0x0008, encode_vec(&new).unwrap()),
            (
                UpdateCode::LargePointer,
                0x0009,
                encode_vec(&large).unwrap(),
            ),
        ]
    }

    /// A slow-path pointer PDU (`messageType`, pad, attribute) on `channel_id`, as one share
    /// control PDU.
    fn slow_path_pointer_frame(channel_id: u16, message_type: u16, attribute: &[u8]) -> Vec<u8> {
        let mut body = message_type.to_le_bytes().to_vec();
        body.extend_from_slice(&[0, 0]);
        body.extend_from_slice(attribute);
        indication(channel_id, share_pdu(body))
    }

    /// A share control PDU holding a share data PDU of type pointer (MS-RDPBCGR 2.2.8.1.1.1.1 and
    /// 2.2.8.1.1.1.2), written by hand because IronRDP does not encode that type.
    fn share_pdu(pointer_body: Vec<u8>) -> Vec<u8> {
        const HEADERS: usize = 6 + 12;
        let total = u16::try_from(HEADERS + pointer_body.len()).unwrap();
        let uncompressed = u16::try_from(4 + pointer_body.len()).unwrap();
        let mut out = Vec::new();
        out.extend_from_slice(&total.to_le_bytes());
        out.extend_from_slice(&0x0017u16.to_le_bytes()); // PDUTYPE_DATAPDU, version 1
        out.extend_from_slice(&1002u16.to_le_bytes()); // pduSource
        out.extend_from_slice(&0x0001_03eau32.to_le_bytes()); // shareId
        out.push(0); // pad1
        out.push(2); // streamId: medium
        out.extend_from_slice(&uncompressed.to_le_bytes());
        out.push(0x1b); // PDUTYPE2_POINTER
        out.push(0); // no compression
        out.extend_from_slice(&0u16.to_le_bytes()); // compressedLength
        out.extend_from_slice(&pointer_body);
        // the bytes are what IronRDP decodes as a pointer PDU
        let decoded: ShareControlHeader = ironrdp_core::decode(&out).unwrap();
        assert!(matches!(
            decoded.share_control_pdu,
            ShareControlPdu::Data(ShareDataHeader {
                share_data_pdu: ShareDataPdu::Pointer(_),
                ..
            })
        ));
        out
    }

    // ---- fragmented fast-path updates ---------------------------------------------------------

    #[test]
    fn the_cap_is_eight_mebibytes_or_twice_the_raw_desktop() {
        assert_eq!(fragment_cap(64, 64), MIN_FRAGMENTED_UPDATE);
        assert_eq!(fragment_cap(1280, 800), MIN_FRAGMENTED_UPDATE);
        assert_eq!(fragment_cap(1920, 1080), 2 * 1920 * 1080 * 4);
        assert_eq!(fragment_cap(3840, 2160), 2 * 3840 * 2160 * 4);
        // the largest desktop the framebuffer accepts
        assert_eq!(fragment_cap(8192, 4320), 2 * 8192 * 4320 * 4);
    }

    #[test]
    fn a_fragmented_update_above_the_cap_is_refused() {
        let mut updates = UpdateCheck::new(IO_CHANNEL, 64, 64);
        assert_eq!(check(&mut updates, &bitmap(Fragmentation::First)), Ok(()));
        // the cap holds exactly 512 fragments of 16 KiB; an endless stream of `Next` is cut there
        let mut held = FRAGMENT as u64;
        while held + FRAGMENT as u64 <= MIN_FRAGMENTED_UPDATE {
            assert_eq!(check(&mut updates, &bitmap(Fragmentation::Next)), Ok(()));
            held += FRAGMENT as u64;
        }
        assert_eq!(held, MIN_FRAGMENTED_UPDATE);
        assert_eq!(
            check(&mut updates, &bitmap(Fragmentation::Next)),
            Err(UpdateOutOfLimits)
        );
        // a `Last` crossing the cap is refused the same way
        let mut updates = UpdateCheck::new(IO_CHANNEL, 64, 64);
        for _ in 0..512 {
            let fragment = if updates.pending.is_none() {
                Fragmentation::First
            } else {
                Fragmentation::Next
            };
            assert_eq!(check(&mut updates, &bitmap(fragment)), Ok(()));
        }
        assert_eq!(
            check(&mut updates, &bitmap(Fragmentation::Last)),
            Err(UpdateOutOfLimits)
        );
    }

    #[test]
    fn a_fragmented_update_below_the_cap_passes_and_the_count_starts_again_after_it() {
        let mut updates = UpdateCheck::new(IO_CHANNEL, 64, 64);
        // three whole updates of exactly the cap in a row: each `Last` ends the count
        for _ in 0..3 {
            assert_eq!(check(&mut updates, &bitmap(Fragmentation::First)), Ok(()));
            for _ in 0..510 {
                assert_eq!(check(&mut updates, &bitmap(Fragmentation::Next)), Ok(()));
            }
            assert_eq!(check(&mut updates, &bitmap(Fragmentation::Last)), Ok(()));
            assert!(updates.pending.is_none());
        }
    }

    #[test]
    fn first_and_single_drop_the_pending_update_like_ironrdp() {
        let mut updates = UpdateCheck::new(IO_CHANNEL, 64, 64);
        for restart in [Fragmentation::First, Fragmentation::Single] {
            assert_eq!(check(&mut updates, &bitmap(Fragmentation::First)), Ok(()));
            for _ in 0..500 {
                assert_eq!(check(&mut updates, &bitmap(Fragmentation::Next)), Ok(()));
            }
            // IronRDP drops what it held: the next sequence is counted from zero
            assert_eq!(check(&mut updates, &bitmap(restart)), Ok(()));
            for _ in 0..500 {
                let fragment = if updates.pending.is_none() {
                    Fragmentation::First
                } else {
                    Fragmentation::Next
                };
                assert_eq!(check(&mut updates, &bitmap(fragment)), Ok(()));
            }
            assert_eq!(check(&mut updates, &bitmap(Fragmentation::Last)), Ok(()));
        }
    }

    #[test]
    fn fragments_ironrdp_refuses_are_left_to_it() {
        let mut updates = UpdateCheck::new(IO_CHANNEL, 64, 64);
        // `Next` without `First`, and a `Next` of another update code: IronRDP ends the session
        assert_eq!(check(&mut updates, &bitmap(Fragmentation::Next)), Ok(()));
        assert!(updates.pending.is_none());
        assert_eq!(check(&mut updates, &bitmap(Fragmentation::First)), Ok(()));
        let other = fast_path_frame(&[update(
            UpdateCode::SurfaceCommands,
            Fragmentation::Next,
            &[0u8; FRAGMENT],
        )]);
        assert_eq!(check(&mut updates, &other), Ok(()));
        assert_eq!(updates.pending.map(|p| p.len), Some(FRAGMENT as u64));
        // a truncated update makes IronRDP discard the pending one
        let mut cut = bitmap(Fragmentation::Next);
        cut.truncate(cut.len() - 100);
        assert_eq!(check(&mut updates, &cut), Ok(()));
        assert!(updates.pending.is_none());
        // and frames that do not decode pass untouched
        assert_eq!(check(&mut updates, &[]), Ok(()));
        assert_eq!(check(&mut updates, &[0x00, 0xff]), Ok(()));
    }

    #[test]
    fn every_update_of_a_frame_is_counted() {
        let mut updates = UpdateCheck::new(IO_CHANNEL, 64, 64);
        assert_eq!(check(&mut updates, &bitmap(Fragmentation::First)), Ok(()));
        // two `Next` updates per frame
        let two = fast_path_frame(&[
            update(
                UpdateCode::Bitmap,
                Fragmentation::Next,
                &[0u8; FRAGMENT / 2],
            ),
            update(
                UpdateCode::Bitmap,
                Fragmentation::Next,
                &[0u8; FRAGMENT / 2],
            ),
        ]);
        for _ in 0..511 {
            assert_eq!(check(&mut updates, &two), Ok(()));
        }
        assert_eq!(check(&mut updates, &two), Err(UpdateOutOfLimits));
    }

    #[test]
    fn the_cap_follows_the_desktop_and_a_reactivation_ends_the_pending_update() {
        // 9 MiB in one update: refused for a small desktop, allowed for 1920 x 1080
        let nine = 9 * 1024 * 1024 / FRAGMENT;
        let run = |updates: &mut UpdateCheck| {
            let mut result = check(updates, &bitmap(Fragmentation::First));
            for _ in 1..nine {
                result = result.and_then(|()| check(updates, &bitmap(Fragmentation::Next)));
            }
            result
        };
        assert_eq!(
            run(&mut UpdateCheck::new(IO_CHANNEL, 1280, 800)),
            Err(UpdateOutOfLimits)
        );
        let mut updates = UpdateCheck::new(IO_CHANNEL, 1280, 800);
        updates.reactivated(IO_CHANNEL, 1920, 1080);
        assert_eq!(run(&mut updates), Ok(()));
        // the reactivation drops what was pending (IronRDP builds a new fast-path processor)
        updates.reactivated(IO_CHANNEL, 1920, 1080);
        assert!(updates.pending.is_none());
    }

    // ---- pointers -----------------------------------------------------------------------------

    #[test]
    fn a_fast_path_pointer_outside_the_cache_is_refused() {
        let mut updates = UpdateCheck::new(IO_CHANNEL, 64, 64);
        for (code, _, data) in pointers(32, 32, 32) {
            let frame = fast_path_frame(&[update(code, Fragmentation::Single, &data)]);
            assert_eq!(
                check(&mut updates, &frame),
                Err(UpdateOutOfLimits),
                "{code:?}"
            );
        }
        for (code, _, data) in pointers(31, 32, 32) {
            let frame = fast_path_frame(&[update(code, Fragmentation::Single, &data)]);
            assert_eq!(check(&mut updates, &frame), Ok(()), "{code:?}");
        }
        // the largest index a u16 holds
        for (code, _, data) in pointers(u16::MAX, 32, 32) {
            let frame = fast_path_frame(&[update(code, Fragmentation::Single, &data)]);
            assert_eq!(check(&mut updates, &frame), Err(UpdateOutOfLimits));
        }
    }

    #[test]
    fn a_fast_path_pointer_above_384_pixels_is_refused() {
        let mut updates = UpdateCheck::new(IO_CHANNEL, 64, 64);
        for (width, height) in [(385, 32), (32, 385), (4096, 62)] {
            for (code, _, data) in pointers(0, width, height) {
                let frame = fast_path_frame(&[update(code, Fragmentation::Single, &data)]);
                let expected = if code == UpdateCode::CachedPointer {
                    // a cached pointer has no size
                    Ok(())
                } else {
                    Err(UpdateOutOfLimits)
                };
                assert_eq!(
                    check(&mut updates, &frame),
                    expected,
                    "{code:?} {width}x{height}"
                );
            }
        }
        for (code, _, data) in pointers(0, 384, 384) {
            let frame = fast_path_frame(&[update(code, Fragmentation::Single, &data)]);
            assert_eq!(check(&mut updates, &frame), Ok(()), "{code:?}");
        }
    }

    #[test]
    fn a_fragmented_pointer_is_judged_by_its_first_fragment() {
        let mut updates = UpdateCheck::new(IO_CHANNEL, 64, 64);
        for (code, _, data) in pointers(40, 32, 32) {
            let frame = fast_path_frame(&[update(code, Fragmentation::First, &data)]);
            assert_eq!(
                check(&mut updates, &frame),
                Err(UpdateOutOfLimits),
                "{code:?}"
            );
        }
        // a first fragment too short for the pointer header is refused: the rest would follow
        // unseen; a whole update that short passes (IronRDP ignores it)
        for (code, _, data) in pointers(0, 32, 32) {
            let short = &data[..1];
            let first = fast_path_frame(&[update(code, Fragmentation::First, short)]);
            assert_eq!(
                check(&mut updates, &first),
                Err(UpdateOutOfLimits),
                "{code:?}"
            );
            let single = fast_path_frame(&[update(code, Fragmentation::Single, short)]);
            assert_eq!(check(&mut updates, &single), Ok(()), "{code:?}");
        }
        // a large pointer in fragments with a whole header passes
        let (_, _, large) = pointers(3, 384, 384).pop().unwrap();
        let first = fast_path_frame(&[update(
            UpdateCode::LargePointer,
            Fragmentation::First,
            &large,
        )]);
        assert_eq!(check(&mut updates, &first), Ok(()));
        let last = fast_path_frame(&[update(
            UpdateCode::LargePointer,
            Fragmentation::Last,
            &[0u8; FRAGMENT],
        )]);
        assert_eq!(check(&mut updates, &last), Ok(()));
    }

    #[test]
    fn a_slow_path_pointer_outside_the_limits_is_refused() {
        let updates = UpdateCheck::new(IO_CHANNEL, 64, 64);
        for (code, message_type, data) in pointers(32, 32, 32) {
            let frame = slow_path_pointer_frame(IO_CHANNEL, message_type, &data);
            assert_eq!(
                updates.slow_path(&frame),
                Err(UpdateOutOfLimits),
                "{code:?}"
            );
        }
        for (code, message_type, data) in pointers(0, 385, 32) {
            let frame = slow_path_pointer_frame(IO_CHANNEL, message_type, &data);
            let expected = if code == UpdateCode::CachedPointer {
                Ok(())
            } else {
                Err(UpdateOutOfLimits)
            };
            assert_eq!(updates.slow_path(&frame), expected, "{code:?}");
        }
        for (code, message_type, data) in pointers(31, 384, 384) {
            let frame = slow_path_pointer_frame(IO_CHANNEL, message_type, &data);
            assert_eq!(updates.slow_path(&frame), Ok(()), "{code:?}");
            // through the public entry point too
            let mut updates = UpdateCheck::new(IO_CHANNEL, 64, 64);
            assert_eq!(updates.check(Action::X224, &frame), Ok(()), "{code:?}");
        }
        // the same bytes on a static channel are no share PDU
        let (_, message_type, data) = pointers(32, 32, 32).remove(0);
        let frame = slow_path_pointer_frame(1004, message_type, &data);
        assert_eq!(updates.slow_path(&frame), Ok(()));
    }

    #[test]
    fn every_concatenated_share_pdu_is_checked() {
        let updates = UpdateCheck::new(IO_CHANNEL, 64, 64);
        let pointer_body = |cache_index: u16| {
            let (_, message_type, data) = pointers(cache_index, 32, 32).remove(0);
            let mut body = message_type.to_le_bytes().to_vec();
            body.extend_from_slice(&[0, 0]);
            body.extend_from_slice(&data);
            share_pdu(body)
        };
        let fine = [pointer_body(1), pointer_body(2)].concat();
        assert_eq!(updates.slow_path(&indication(IO_CHANNEL, fine)), Ok(()));
        let second_refused = [pointer_body(1), pointer_body(32)].concat();
        assert_eq!(
            updates.slow_path(&indication(IO_CHANNEL, second_refused)),
            Err(UpdateOutOfLimits)
        );
        // frames that do not decode pass
        assert_eq!(updates.slow_path(&[0xde, 0xad]), Ok(()));
        assert_eq!(
            updates.slow_path(&indication(IO_CHANNEL, vec![0x01])),
            Ok(())
        );
    }

    #[test]
    fn a_compressed_update_is_left_to_ironrdp() {
        // without a decompressor IronRDP ends the session on it; the check does not count it
        let mut updates = UpdateCheck::new(IO_CHANNEL, 64, 64);
        let (_, _, data) = pointers(40, 32, 32).remove(0);
        let compressed = encode_vec(&FastPathUpdatePdu {
            fragmentation: Fragmentation::Single,
            update_code: UpdateCode::ColorPointer,
            compression_flags: Some(CompressionFlags::COMPRESSED),
            compression_type: Some(CompressionType::K8),
            data: &data,
        })
        .unwrap();
        assert_eq!(check(&mut updates, &fast_path_frame(&[compressed])), Ok(()));
    }
}
