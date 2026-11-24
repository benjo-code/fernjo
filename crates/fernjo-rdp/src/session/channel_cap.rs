//! A cap on the length a server may announce for one virtual channel message.
//!
//! IronRDP reassembles a chunked static channel message (`ironrdp-svc`, `ChunkProcessor`) and a
//! fragmented dynamic channel message (`ironrdp-dvc`, `CompleteData`) into a buffer that grows
//! until it reaches the total length announced in the first chunk: the `length` field of the
//! channel PDU header (a `u32`) or of a DVC "data first" PDU. Nothing caps that announcement, so a
//! hostile server could make the client buffer up to 4 GiB. [`check_frame`] looks at a frame
//! before it reaches the active stage and refuses an announcement above
//! [`MAX_CHANNEL_MESSAGE`]; the session then ends with `CloseReason::ChannelTooLarge`.
//!
//! The check is conservative: anything it cannot decode passes, and IronRDP reports its own error
//! for it. Fast-path frames, the I/O and message channels and every chunk that does not start a
//! message pass untouched. The largest legitimate message is clipboard text: 1 MiB of text is
//! about 2 MiB of UTF-16, well below the cap.

use ironrdp_core::{ReadCursor, decode_cursor};
use ironrdp_pdu::Action;
use ironrdp_pdu::rdp::vc::{ChannelControlFlags, ChannelPduHeader};

use crate::error::ChannelTooLarge;

/// The largest total length a server may announce for one channel message.
pub(crate) const MAX_CHANNEL_MESSAGE: u32 = 4 * 1024 * 1024;

/// DVC command "data first" (MS-RDPEDYC 2.2.3.1).
const DVC_DATA_FIRST: u8 = 0x02;
/// DVC command "data first compressed" (MS-RDPEDYC 2.2.3.3); its length is the uncompressed one.
const DVC_DATA_FIRST_COMPRESSED: u8 = 0x06;

/// The MCS channel ids of the session's static channels.
#[derive(Debug, Clone, Default)]
pub(crate) struct ChannelIds {
    /// Every static virtual channel (cliprdr, drdynvc, ...); not the I/O or message channel.
    pub(crate) statics: Vec<u16>,
    /// The static channel that carries the dynamic channels, when the session has one.
    pub(crate) drdynvc: Option<u16>,
}

/// Refuses a frame that starts a channel message announced above [`MAX_CHANNEL_MESSAGE`].
pub(crate) fn check_frame(
    action: Action,
    payload: &[u8],
    channels: &ChannelIds,
) -> Result<(), ChannelTooLarge> {
    if action != Action::X224 {
        return Ok(());
    }
    let Ok(data) = ironrdp_pdu::mcs::decode_send_data_indication(payload) else {
        return Ok(());
    };
    if !channels.statics.contains(&data.channel_id) {
        return Ok(());
    }
    let mut cursor = ReadCursor::new(data.user_data);
    let Ok(header) = decode_cursor::<ChannelPduHeader>(&mut cursor) else {
        return Ok(());
    };
    let first = header.flags.contains(ChannelControlFlags::FLAG_FIRST);
    let last = header.flags.contains(ChannelControlFlags::FLAG_LAST);
    if first && header.length > MAX_CHANNEL_MESSAGE {
        return Err(ChannelTooLarge);
    }

    if channels.drdynvc == Some(data.channel_id)
        && !header
            .flags
            .contains(ChannelControlFlags::PACKET_COMPRESSED)
    {
        let chunk = cursor.remaining();
        // A DVC PDU starts a static channel message: a chunk with the first flag, or one without
        // either flag that holds the whole message (IronRDP takes that as unfragmented).
        let starts =
            first || (!last && u32::try_from(chunk.len()).is_ok_and(|len| len == header.length));
        if starts && dvc_data_first_length(chunk).is_some_and(|len| len > MAX_CHANNEL_MESSAGE) {
            return Err(ChannelTooLarge);
        }
    }
    Ok(())
}

/// The total length a DVC "data first" PDU announces, `None` for any other PDU or a short one.
fn dvc_data_first_length(pdu: &[u8]) -> Option<u32> {
    let (&byte, rest) = pdu.split_first()?;
    let command = byte >> 4;
    if command != DVC_DATA_FIRST && command != DVC_DATA_FIRST_COMPRESSED {
        return None;
    }
    let field_len = |bits: u8| match bits {
        0 => Some(1),
        1 => Some(2),
        2 => Some(4),
        _ => None,
    };
    let id_len = field_len(byte & 0b11)?;
    let length_len = field_len((byte >> 2) & 0b11)?;
    let field = rest.get(id_len..id_len + length_len)?;
    let mut bytes = [0u8; 4];
    bytes[..length_len].copy_from_slice(field);
    Some(u32::from_le_bytes(bytes))
}

#[cfg(test)]
pub(super) mod tests {
    use std::borrow::Cow;

    use ironrdp_core::encode_vec;
    use ironrdp_dvc::pdu::{DataFirstPdu, DrdynvcDataPdu, FieldType};
    use ironrdp_pdu::mcs::SendDataIndication;
    use ironrdp_pdu::x224::X224;

    use super::*;

    const IO_CHANNEL: u16 = 1003;
    const CLIPRDR: u16 = 1004;
    const DRDYNVC: u16 = 1005;

    fn channels() -> ChannelIds {
        ChannelIds {
            statics: vec![CLIPRDR, DRDYNVC],
            drdynvc: Some(DRDYNVC),
        }
    }

    /// An X.224 frame carrying `user_data` on `channel_id` as the server sends it.
    pub(in crate::session) fn indication(channel_id: u16, user_data: Vec<u8>) -> Vec<u8> {
        encode_vec(&X224(SendDataIndication {
            initiator_id: 1002,
            channel_id,
            user_data: Cow::Owned(user_data),
        }))
        .unwrap()
    }

    /// A static channel chunk: the channel PDU header followed by `data`.
    pub(in crate::session) fn chunk(
        length: u32,
        flags: ChannelControlFlags,
        data: &[u8],
    ) -> Vec<u8> {
        let mut out = encode_vec(&ChannelPduHeader { length, flags }).unwrap();
        out.extend_from_slice(data);
        out
    }

    fn first_chunk_on(channel_id: u16, length: u32) -> Vec<u8> {
        indication(
            channel_id,
            chunk(length, ChannelControlFlags::FLAG_FIRST, &[0u8; 1600]),
        )
    }

    fn data_first(total: u32) -> Vec<u8> {
        let pdu = DrdynvcDataPdu::DataFirst(
            DataFirstPdu::new(7, total, vec![0u8; 1500]).with_sp_type(FieldType::U32),
        );
        encode_vec(&pdu).unwrap()
    }

    #[test]
    fn a_static_channel_message_announced_above_the_cap_is_refused() {
        let frame = first_chunk_on(CLIPRDR, MAX_CHANNEL_MESSAGE + 1);
        assert_eq!(
            check_frame(Action::X224, &frame, &channels()),
            Err(ChannelTooLarge)
        );
        // exactly at the cap is still allowed
        let frame = first_chunk_on(CLIPRDR, MAX_CHANNEL_MESSAGE);
        assert_eq!(check_frame(Action::X224, &frame, &channels()), Ok(()));
    }

    #[test]
    fn a_two_mebibyte_clipboard_message_passes() {
        // 1 MiB of ASCII clipboard text is about 2 MiB of UTF-16 on the wire
        let length = 2 * 1024 * 1024 + 64;
        let frame = first_chunk_on(CLIPRDR, length);
        assert_eq!(check_frame(Action::X224, &frame, &channels()), Ok(()));
        // the chunks after the first one announce the same total but carry no first flag
        let middle = indication(
            CLIPRDR,
            chunk(length, ChannelControlFlags::empty(), &[0u8; 1600]),
        );
        assert_eq!(check_frame(Action::X224, &middle, &channels()), Ok(()));
    }

    #[test]
    fn fast_path_frames_and_non_channel_pdus_pass() {
        let huge_first = chunk(u32::MAX, ChannelControlFlags::FLAG_FIRST, &[0u8; 16]);
        // fast path is graphics and pointer updates, never channel data
        assert_eq!(
            check_frame(Action::FastPath, &[0xff; 64], &channels()),
            Ok(())
        );
        // the I/O channel carries share PDUs, which have no channel PDU header: the same bytes
        // that would be refused on a static channel pass there
        let io = indication(IO_CHANNEL, huge_first.clone());
        assert_eq!(check_frame(Action::X224, &io, &channels()), Ok(()));
        // and so does a channel id this session does not know (IronRDP reports it itself)
        let unknown = indication(1010, huge_first);
        assert_eq!(check_frame(Action::X224, &unknown, &channels()), Ok(()));
    }

    #[test]
    fn a_dynamic_channel_data_first_above_the_cap_is_refused() {
        // one static chunk (first and last) holding a DVC "data first" PDU
        let refused = data_first(MAX_CHANNEL_MESSAGE + 1);
        let frame = indication(
            DRDYNVC,
            chunk(
                u32::try_from(refused.len()).unwrap(),
                ChannelControlFlags::FLAG_FIRST | ChannelControlFlags::FLAG_LAST,
                &refused,
            ),
        );
        assert_eq!(
            check_frame(Action::X224, &frame, &channels()),
            Err(ChannelTooLarge)
        );

        // without the flags, the chunk is still a whole message for IronRDP
        let frame = indication(
            DRDYNVC,
            chunk(
                u32::try_from(refused.len()).unwrap(),
                ChannelControlFlags::empty(),
                &refused,
            ),
        );
        assert_eq!(
            check_frame(Action::X224, &frame, &channels()),
            Err(ChannelTooLarge)
        );

        // a large but allowed DVC message passes
        let allowed = data_first(2 * 1024 * 1024);
        let frame = indication(
            DRDYNVC,
            chunk(
                u32::try_from(allowed.len()).unwrap(),
                ChannelControlFlags::FLAG_FIRST | ChannelControlFlags::FLAG_LAST,
                &allowed,
            ),
        );
        assert_eq!(check_frame(Action::X224, &frame, &channels()), Ok(()));

        // the same bytes on another static channel are not a DVC PDU
        let frame = indication(
            CLIPRDR,
            chunk(
                u32::try_from(refused.len()).unwrap(),
                ChannelControlFlags::FLAG_FIRST | ChannelControlFlags::FLAG_LAST,
                &refused,
            ),
        );
        assert_eq!(check_frame(Action::X224, &frame, &channels()), Ok(()));
    }

    #[test]
    fn a_truncated_frame_is_left_to_ironrdp() {
        let full = first_chunk_on(CLIPRDR, MAX_CHANNEL_MESSAGE + 1);
        // cut inside the TPKT, X.224, MCS or channel headers: nothing decodes, nothing is refused
        for len in [0, 3, 7, 12, 20] {
            assert_eq!(
                check_frame(Action::X224, &full[..len], &channels()),
                Ok(()),
                "cut at {len}"
            );
        }
        assert_eq!(
            check_frame(Action::X224, &[0xde, 0xad, 0xbe, 0xef], &channels()),
            Ok(())
        );
        // a DVC "data first" cut before its length field passes too
        let frame = indication(
            DRDYNVC,
            chunk(2, ChannelControlFlags::FLAG_FIRST, &[0x2a, 0x07]),
        );
        assert_eq!(check_frame(Action::X224, &frame, &channels()), Ok(()));
    }
}
