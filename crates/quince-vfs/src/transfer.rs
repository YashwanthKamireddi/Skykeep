//! How a request for a file's bytes is answered on Windows (Cloud Files): the pieces handed to
//! `CfExecute(TRANSFER_DATA)`. Windows takes a piece only if it starts on a 4 KB boundary and is
//! a whole number of 4 KB blocks, except one that ends at the end of the file. Pure arithmetic,
//! kept apart from the Windows calls so it is proved here for every awkward size.

/// The block Windows transfers in.
pub const BLOCK: u64 = 4096;

/// The pieces that answer a request for `len` bytes at `offset` of a file `size` long, each at
/// most `most` bytes (a multiple of [`BLOCK`]): from `offset` rounded down to a block, to the
/// request's end rounded up to a block or the file's end, whichever comes first. Every byte
/// asked for is in exactly one piece, so the request is done when the last piece is sent —
/// never before, which is how a short file was left waiting in Nextcloud (#10845: "done" was
/// said before every byte had gone through). A file of no bytes is one empty piece at its end.
/// Nothing for a request that starts past the end: the caller fails it, whole.
#[must_use]
pub fn pieces(size: u64, offset: u64, len: u64, most: u64) -> Vec<(u64, u64)> {
    debug_assert!(most >= BLOCK && most.is_multiple_of(BLOCK));
    if size == 0 {
        return if offset == 0 {
            vec![(0, 0)]
        } else {
            Vec::new()
        };
    }
    if offset >= size {
        return Vec::new();
    }
    let start = offset / BLOCK * BLOCK;
    let wanted_end = offset.saturating_add(len.max(1)).min(size);
    let end = wanted_end.div_ceil(BLOCK).saturating_mul(BLOCK).min(size);
    let mut out = Vec::new();
    let mut at = start;
    while at < end {
        let to = (at + most).min(end);
        out.push((at, to - at));
        at = to;
    }
    out
}

/// The first piece read from Apple for a request: small, so a slow line hands Windows bytes
/// well inside the minute it waits for any.
pub const FIRST_PIECE: u64 = 256 * 1024;

/// The smallest piece: below this the line is too slow for anything to arrive in time.
pub const LEAST_PIECE: u64 = 64 * 1024;

/// The next piece's size, after one of `step` bytes took `took`: doubled (to at most `most`)
/// when it came quickly, halved (to at least [`LEAST_PIECE`]) when it came slowly, so each piece
/// arrives in a few seconds on any line. Windows fails a request that receives nothing for 60
/// seconds, and a fixed 4 MB piece took longer than that below about 70 KB/s. Always a whole
/// number of [`BLOCK`]s, as every piece but a file's last must be.
#[must_use]
pub fn next_piece(step: u64, took: std::time::Duration, most: u64) -> u64 {
    let next = if took < std::time::Duration::from_secs(4) {
        step.saturating_mul(2)
    } else if took > std::time::Duration::from_secs(12) {
        step / 2
    } else {
        step
    };
    next.clamp(LEAST_PIECE, most.max(LEAST_PIECE)) / BLOCK * BLOCK
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every piece starts on a block and is whole blocks unless it ends the file, the pieces
    /// meet end to end, and together they hold every byte asked for — at the sizes and
    /// offsets around a block that broke other clients.
    #[test]
    fn every_request_is_answered_whole_and_aligned() {
        let most = 4 * BLOCK;
        for size in [
            1,
            2462,
            4095,
            4096,
            4097,
            8191,
            8192,
            8193,
            5 * BLOCK + 7,
            64 * BLOCK,
        ] {
            for offset in [0, 1, 4095, 4096, 4097, size.saturating_sub(1)] {
                if offset >= size {
                    continue;
                }
                for len in [1, 4095, 4096, size] {
                    let p = pieces(size, offset, len, most);
                    let (first, last) = (p[0], p[p.len() - 1]);
                    assert_eq!(first.0 % BLOCK, 0);
                    assert!(first.0 <= offset);
                    assert!(
                        last.0 + last.1 >= (offset + len).min(size),
                        "{size} {offset} {len}"
                    );
                    for w in p.windows(2) {
                        assert_eq!(w[0].0 + w[0].1, w[1].0);
                    }
                    for &(at, n) in &p {
                        assert!(n <= most && n > 0);
                        assert!(
                            n % BLOCK == 0 || at + n == size,
                            "{size} {offset} {len}: {at}+{n}"
                        );
                    }
                }
            }
        }
        assert_eq!(pieces(0, 0, 10, most), [(0, 0)]);
        assert!(pieces(100, 100, 1, most).is_empty());
    }

    /// Pieces grow on a fast line to the largest, shrink on a slow one to the smallest, stay
    /// put in between, and are always whole blocks.
    #[test]
    fn pieces_follow_the_line() {
        use std::time::Duration;
        let most = 4 << 20;
        let mut step = FIRST_PIECE;
        for _ in 0..10 {
            step = next_piece(step, Duration::from_millis(300), most);
        }
        assert_eq!(step, most, "a fast line reaches the largest piece");
        for _ in 0..10 {
            step = next_piece(step, Duration::from_secs(30), most);
        }
        assert_eq!(step, LEAST_PIECE, "a slow line comes down to the smallest");
        assert_eq!(
            next_piece(FIRST_PIECE, Duration::from_secs(8), most),
            FIRST_PIECE
        );
        // At 20 KB/s a piece of the smallest size takes about three seconds, well inside the
        // minute; the old fixed 4 MB took three and a half minutes.
        const _: () = assert!(LEAST_PIECE / (20 * 1024) < 60);
        for (step, took) in [(FIRST_PIECE, 1), (LEAST_PIECE, 30), (most, 1)] {
            assert_eq!(next_piece(step, Duration::from_secs(took), most) % BLOCK, 0);
        }
    }
}
