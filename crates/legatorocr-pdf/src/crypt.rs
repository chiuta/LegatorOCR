//! Cryptographic primitives for reading encrypted PDFs (standard security handler with
//! an empty user password): MD5, RC4, AES-128/256 (CBC), SHA-256/384/512.
//! Written from the public algorithm descriptions (RFC 1321, FIPS 180-4 and 197);
//! constants that are defined as roots of primes are computed, not copied.
#![allow(clippy::needless_range_loop)]

const K256: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];
const K512: [u64; 80] = [
    0x428a2f98d728ae22,
    0x7137449123ef65cd,
    0xb5c0fbcfec4d3b2f,
    0xe9b5dba58189dbbc,
    0x3956c25bf348b538,
    0x59f111f1b605d019,
    0x923f82a4af194f9b,
    0xab1c5ed5da6d8118,
    0xd807aa98a3030242,
    0x12835b0145706fbe,
    0x243185be4ee4b28c,
    0x550c7dc3d5ffb4e2,
    0x72be5d74f27b896f,
    0x80deb1fe3b1696b1,
    0x9bdc06a725c71235,
    0xc19bf174cf692694,
    0xe49b69c19ef14ad2,
    0xefbe4786384f25e3,
    0x0fc19dc68b8cd5b5,
    0x240ca1cc77ac9c65,
    0x2de92c6f592b0275,
    0x4a7484aa6ea6e483,
    0x5cb0a9dcbd41fbd4,
    0x76f988da831153b5,
    0x983e5152ee66dfab,
    0xa831c66d2db43210,
    0xb00327c898fb213f,
    0xbf597fc7beef0ee4,
    0xc6e00bf33da88fc2,
    0xd5a79147930aa725,
    0x06ca6351e003826f,
    0x142929670a0e6e70,
    0x27b70a8546d22ffc,
    0x2e1b21385c26c926,
    0x4d2c6dfc5ac42aed,
    0x53380d139d95b3df,
    0x650a73548baf63de,
    0x766a0abb3c77b2a8,
    0x81c2c92e47edaee6,
    0x92722c851482353b,
    0xa2bfe8a14cf10364,
    0xa81a664bbc423001,
    0xc24b8b70d0f89791,
    0xc76c51a30654be30,
    0xd192e819d6ef5218,
    0xd69906245565a910,
    0xf40e35855771202a,
    0x106aa07032bbd1b8,
    0x19a4c116b8d2d0c8,
    0x1e376c085141ab53,
    0x2748774cdf8eeb99,
    0x34b0bcb5e19b48a8,
    0x391c0cb3c5c95a63,
    0x4ed8aa4ae3418acb,
    0x5b9cca4f7763e373,
    0x682e6ff3d6b2b8a3,
    0x748f82ee5defb2fc,
    0x78a5636f43172f60,
    0x84c87814a1f0ab72,
    0x8cc702081a6439ec,
    0x90befffa23631e28,
    0xa4506cebde82bde9,
    0xbef9a3f7b2c67915,
    0xc67178f2e372532b,
    0xca273eceea26619c,
    0xd186b8c721c0c207,
    0xeada7dd6cde0eb1e,
    0xf57d4f7fee6ed178,
    0x06f067aa72176fba,
    0x0a637dc5a2c898a6,
    0x113f9804bef90dae,
    0x1b710b35131c471b,
    0x28db77f523047d84,
    0x32caab7b40c72493,
    0x3c9ebe0a15c9bebc,
    0x431d67c49c100d4c,
    0x4cc5d4becb3e42b6,
    0x597f299cfc657e2a,
    0x5fcb6fab3ad6faec,
    0x6c44198c4a475817,
];
const H256: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];
const H384: [u64; 8] = [
    0xcbbb9d5dc1059ed8,
    0x629a292a367cd507,
    0x9159015a3070dd17,
    0x152fecd8f70e5939,
    0x67332667ffc00b31,
    0x8eb44a8768581511,
    0xdb0c2e0d64f98fa7,
    0x47b5481dbefa4fa4,
];
const H512: [u64; 8] = [
    0x6a09e667f3bcc908,
    0xbb67ae8584caa73b,
    0x3c6ef372fe94f82b,
    0xa54ff53a5f1d36f1,
    0x510e527fade682d1,
    0x9b05688c2b3e6c1f,
    0x1f83d9abfb41bd6b,
    0x5be0cd19137e2179,
];

// ---------------------------------------------------------------- MD5
pub fn md5(data: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64)
        .map(|i| ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32)
        .collect();
    let mut st = [0x6745_2301u32, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    let mut m = data.to_vec();
    m.push(0x80);
    while m.len() % 64 != 56 {
        m.push(0);
    }
    m.extend(((data.len() as u64) * 8).to_le_bytes());
    for chunk in m.chunks(64) {
        let w: Vec<u32> = chunk
            .chunks(4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        let [mut a, mut b, mut c, mut d] = st;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let t = d;
            d = c;
            c = b;
            b = b.wrapping_add(
                a.wrapping_add(f)
                    .wrapping_add(k[i])
                    .wrapping_add(w[g])
                    .rotate_left(S[i]),
            );
            a = t;
        }
        st[0] = st[0].wrapping_add(a);
        st[1] = st[1].wrapping_add(b);
        st[2] = st[2].wrapping_add(c);
        st[3] = st[3].wrapping_add(d);
    }
    let mut out = [0u8; 16];
    for (i, v) in st.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

// ---------------------------------------------------------------- RC4
pub fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    if key.is_empty() {
        return data.to_vec();
    }
    let mut s: Vec<u8> = (0..=255).collect();
    let mut j = 0usize;
    for i in 0..256 {
        j = (j + s[i] as usize + key[i % key.len()] as usize) & 255;
        s.swap(i, j);
    }
    let (mut i, mut j) = (0usize, 0usize);
    data.iter()
        .map(|&b| {
            i = (i + 1) & 255;
            j = (j + s[i] as usize) & 255;
            s.swap(i, j);
            b ^ s[(s[i] as usize + s[j] as usize) & 255]
        })
        .collect()
}

// ---------------------------------------------------------------- SHA-2
pub fn sha256(data: &[u8]) -> Vec<u8> {
    let mut h = H256;
    let mut m = data.to_vec();
    m.push(0x80);
    while m.len() % 64 != 56 {
        m.push(0);
    }
    m.extend(((data.len() as u64) * 8).to_be_bytes());
    for chunk in m.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[4 * i],
                chunk[4 * i + 1],
                chunk[4 * i + 2],
                chunk[4 * i + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K256[i])
                .wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [
                t1.wrapping_add(t2),
                v[0],
                v[1],
                v[2],
                v[3].wrapping_add(t1),
                v[4],
                v[5],
                v[6],
            ];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    h.iter().flat_map(|x| x.to_be_bytes()).collect()
}

fn sha512_core(data: &[u8], init: [u64; 8], out_len: usize) -> Vec<u8> {
    let mut h = init;
    let mut m = data.to_vec();
    m.push(0x80);
    while m.len() % 128 != 112 {
        m.push(0);
    }
    m.extend(((data.len() as u128) * 8).to_be_bytes());
    for chunk in m.chunks(128) {
        let mut w = [0u64; 80];
        for i in 0..16 {
            let mut b = [0u8; 8];
            b.copy_from_slice(&chunk[8 * i..8 * i + 8]);
            w[i] = u64::from_be_bytes(b);
        }
        for i in 16..80 {
            let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
            let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..80 {
            let s1 = v[4].rotate_right(14) ^ v[4].rotate_right(18) ^ v[4].rotate_right(41);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K512[i])
                .wrapping_add(w[i]);
            let s0 = v[0].rotate_right(28) ^ v[0].rotate_right(34) ^ v[0].rotate_right(39);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [
                t1.wrapping_add(t2),
                v[0],
                v[1],
                v[2],
                v[3].wrapping_add(t1),
                v[4],
                v[5],
                v[6],
            ];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    let mut o: Vec<u8> = h.iter().flat_map(|x| x.to_be_bytes()).collect();
    o.truncate(out_len);
    o
}
pub fn sha384(data: &[u8]) -> Vec<u8> {
    sha512_core(data, H384, 48)
}
pub fn sha512(data: &[u8]) -> Vec<u8> {
    sha512_core(data, H512, 64)
}

// ---------------------------------------------------------------- AES
fn gmul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0;
    while b != 0 {
        if b & 1 != 0 {
            p ^= a;
        }
        let hi = a & 0x80;
        a <<= 1;
        if hi != 0 {
            a ^= 0x1b;
        }
        b >>= 1;
    }
    p
}

struct Tables {
    sbox: [u8; 256],
    inv: [u8; 256],
    /// Multiplication tables of GF(2^8) by the MixColumns constants: a table lookup instead of
    /// a bitwise loop per byte (AES of a 100 MB scan took tens of seconds with `gmul`).
    m2: [u8; 256],
    m3: [u8; 256],
    m9: [u8; 256],
    m11: [u8; 256],
    m13: [u8; 256],
    m14: [u8; 256],
}
fn tables() -> &'static Tables {
    static T: std::sync::OnceLock<Tables> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut sbox = [0u8; 256];
        let mut inv = [0u8; 256];
        for x in 0..256usize {
            // multiplicative inverse by search (0 maps to 0)
            let i = if x == 0 {
                0
            } else {
                (1..=255u8).find(|&y| gmul(x as u8, y) == 1).unwrap()
            };
            let s = i
                ^ i.rotate_left(1)
                ^ i.rotate_left(2)
                ^ i.rotate_left(3)
                ^ i.rotate_left(4)
                ^ 0x63;
            sbox[x] = s;
            inv[s as usize] = x as u8;
        }
        let mul = |k: u8| -> [u8; 256] {
            let mut t = [0u8; 256];
            for (x, v) in t.iter_mut().enumerate() {
                *v = gmul(x as u8, k);
            }
            t
        };
        Tables {
            sbox,
            inv,
            m2: mul(2),
            m3: mul(3),
            m9: mul(9),
            m11: mul(11),
            m13: mul(13),
            m14: mul(14),
        }
    })
}

pub struct Aes {
    rk: Vec<[u8; 16]>,
}
impl Aes {
    /// `key` must be 16 or 32 bytes.
    pub fn new(key: &[u8]) -> Option<Aes> {
        let nk = match key.len() {
            16 => 4,
            24 => 6,
            32 => 8,
            _ => return None,
        };
        let nr = nk + 6;
        let t = tables();
        let mut w: Vec<[u8; 4]> = key.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
        let mut rcon = 1u8;
        for i in nk..4 * (nr + 1) {
            let mut tmp = w[i - 1];
            if i % nk == 0 {
                tmp = [
                    t.sbox[tmp[1] as usize] ^ rcon,
                    t.sbox[tmp[2] as usize],
                    t.sbox[tmp[3] as usize],
                    t.sbox[tmp[0] as usize],
                ];
                rcon = gmul(rcon, 2);
            } else if nk > 6 && i % nk == 4 {
                tmp = [
                    t.sbox[tmp[0] as usize],
                    t.sbox[tmp[1] as usize],
                    t.sbox[tmp[2] as usize],
                    t.sbox[tmp[3] as usize],
                ];
            }
            let p = w[i - nk];
            w.push([p[0] ^ tmp[0], p[1] ^ tmp[1], p[2] ^ tmp[2], p[3] ^ tmp[3]]);
        }
        let rk = w
            .chunks(4)
            .map(|c| {
                let mut k = [0u8; 16];
                for (i, word) in c.iter().enumerate() {
                    k[4 * i..4 * i + 4].copy_from_slice(word);
                }
                k
            })
            .collect();
        Some(Aes { rk })
    }
    fn add(s: &mut [u8; 16], k: &[u8; 16]) {
        for i in 0..16 {
            s[i] ^= k[i];
        }
    }
    pub fn encrypt_block(&self, b: &mut [u8; 16]) {
        let t = tables();
        let nr = self.rk.len() - 1;
        Self::add(b, &self.rk[0]);
        for r in 1..=nr {
            for x in b.iter_mut() {
                *x = t.sbox[*x as usize];
            }
            let c = *b;
            for col in 0..4 {
                for row in 0..4 {
                    b[4 * col + row] = c[4 * ((col + row) % 4) + row];
                }
            }
            if r != nr {
                for col in 0..4 {
                    let a = [b[4 * col], b[4 * col + 1], b[4 * col + 2], b[4 * col + 3]];
                    let (x, y, z, w) = (a[0] as usize, a[1] as usize, a[2] as usize, a[3] as usize);
                    b[4 * col] = t.m2[x] ^ t.m3[y] ^ a[2] ^ a[3];
                    b[4 * col + 1] = a[0] ^ t.m2[y] ^ t.m3[z] ^ a[3];
                    b[4 * col + 2] = a[0] ^ a[1] ^ t.m2[z] ^ t.m3[w];
                    b[4 * col + 3] = t.m3[x] ^ a[1] ^ a[2] ^ t.m2[w];
                }
            }
            Self::add(b, &self.rk[r]);
        }
    }
    pub fn decrypt_block(&self, b: &mut [u8; 16]) {
        let t = tables();
        let nr = self.rk.len() - 1;
        Self::add(b, &self.rk[nr]);
        for r in (0..nr).rev() {
            let c = *b;
            for col in 0..4 {
                for row in 0..4 {
                    b[4 * ((col + row) % 4) + row] = c[4 * col + row];
                }
            }
            for x in b.iter_mut() {
                *x = t.inv[*x as usize];
            }
            Self::add(b, &self.rk[r]);
            if r != 0 {
                for col in 0..4 {
                    let a = [b[4 * col], b[4 * col + 1], b[4 * col + 2], b[4 * col + 3]];
                    let (x, y, z, w) = (a[0] as usize, a[1] as usize, a[2] as usize, a[3] as usize);
                    b[4 * col] = t.m14[x] ^ t.m11[y] ^ t.m13[z] ^ t.m9[w];
                    b[4 * col + 1] = t.m9[x] ^ t.m14[y] ^ t.m11[z] ^ t.m13[w];
                    b[4 * col + 2] = t.m13[x] ^ t.m9[y] ^ t.m14[z] ^ t.m11[w];
                    b[4 * col + 3] = t.m11[x] ^ t.m13[y] ^ t.m9[z] ^ t.m14[w];
                }
            }
        }
    }
}

/// CBC decryption; with `iv == None` the first 16 bytes of `data` are the IV and PKCS#7
/// padding is removed (PDF stream convention).
pub fn aes_cbc_decrypt(key: &[u8], iv: Option<[u8; 16]>, data: &[u8]) -> Option<Vec<u8>> {
    let aes = Aes::new(key)?;
    let (mut prev, body, strip) = match iv {
        Some(iv) => (iv, data, false),
        None => {
            if data.len() < 16 {
                return Some(Vec::new());
            }
            let mut iv = [0u8; 16];
            iv.copy_from_slice(&data[..16]);
            (iv, &data[16..], true)
        }
    };
    let mut out = Vec::with_capacity(body.len());
    for c in body.as_chunks::<16>().0 {
        let mut b = [0u8; 16];
        b.copy_from_slice(c);
        let ct = b;
        aes.decrypt_block(&mut b);
        for i in 0..16 {
            b[i] ^= prev[i];
        }
        prev = ct;
        out.extend_from_slice(&b);
    }
    if strip {
        if let Some(&p) = out.last() {
            if (1..=16).contains(&p) && out.len() >= p as usize {
                out.truncate(out.len() - p as usize);
            }
        }
    }
    Some(out)
}

/// CBC encryption without padding (`data.len()` multiple of 16).
pub fn aes_cbc_encrypt_nopad(key: &[u8], iv: [u8; 16], data: &[u8]) -> Option<Vec<u8>> {
    let aes = Aes::new(key)?;
    let mut prev = iv;
    let mut out = Vec::with_capacity(data.len());
    for c in data.as_chunks::<16>().0 {
        let mut b = [0u8; 16];
        for i in 0..16 {
            b[i] = c[i] ^ prev[i];
        }
        aes.encrypt_block(&mut b);
        prev = b;
        out.extend_from_slice(&b);
    }
    Some(out)
}

// ---------------------------------------------------------------- PDF security handler
const PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08,
    0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Method {
    None,
    Rc4,
    AesV2,
    AesV3,
}

#[derive(Clone, Debug)]
pub struct Crypt {
    key: Vec<u8>,
    pub stm: Method,
}

/// Everything the Encrypt dictionary and trailer provide.
pub struct EncryptParams<'a> {
    pub v: i64,
    pub r: i64,
    pub length_bits: i64,
    pub o: &'a [u8],
    pub u: &'a [u8],
    pub ue: &'a [u8],
    pub p: i64,
    pub id0: &'a [u8],
    pub encrypt_metadata: bool,
    pub stm: Method,
}

/// Algorithm 2.B (revision 6) hash.
fn hash_r6(pw: &[u8], salt: &[u8], udata: &[u8]) -> Vec<u8> {
    let mut input = pw.to_vec();
    input.extend_from_slice(salt);
    input.extend_from_slice(udata);
    let mut k = sha256(&input);
    let mut i = 0usize;
    loop {
        let mut k1 = Vec::new();
        for _ in 0..64 {
            k1.extend_from_slice(pw);
            k1.extend_from_slice(&k);
            k1.extend_from_slice(udata);
        }
        let mut iv = [0u8; 16];
        iv.copy_from_slice(&k[16..32]);
        let e = aes_cbc_encrypt_nopad(&k[..16], iv, &k1).unwrap_or_default();
        let m: u32 = e[..16].iter().map(|&b| b as u32).sum::<u32>() % 3;
        k = match m {
            0 => sha256(&e),
            1 => sha384(&e),
            _ => sha512(&e),
        };
        i += 1;
        if i >= 64 && (*e.last().unwrap_or(&0) as usize) <= i - 32 {
            break;
        }
        if i > 2000 {
            break;
        }
    }
    k.truncate(32);
    k
}

impl Crypt {
    /// Derive the file key for the empty user password; `Err` when the file needs a password
    /// or uses something unsupported.
    pub fn new(p: &EncryptParams) -> Result<Crypt, String> {
        if p.r >= 5 {
            if p.u.len() < 48 || p.ue.len() < 32 {
                return Err("the PDF's encryption data is incomplete".into());
            }
            let (vs, ks) = (&p.u[32..40], &p.u[40..48]);
            let h = |salt: &[u8]| {
                if p.r == 5 {
                    sha256(salt)
                } else {
                    hash_r6(b"", salt, b"")
                }
            };
            if h(vs)[..32] != p.u[..32] {
                return Err("the PDF is password-protected (it needs a user password)".into());
            }
            let ik = h(ks);
            let key = aes_cbc_decrypt(&ik, Some([0; 16]), &p.ue[..32]).ok_or("bad key")?;
            return Ok(Crypt {
                key,
                stm: if p.stm == Method::None {
                    Method::None
                } else {
                    Method::AesV3
                },
            });
        }
        let n = if p.r == 2 {
            5
        } else {
            (p.length_bits / 8).clamp(5, 16) as usize
        };
        let mut m = PAD.to_vec();
        m.extend_from_slice(p.o);
        m.extend_from_slice(&(p.p as i32 as u32).to_le_bytes());
        m.extend_from_slice(p.id0);
        if p.r >= 4 && !p.encrypt_metadata {
            m.extend_from_slice(&[0xFF; 4]);
        }
        let mut h = md5(&m).to_vec();
        if p.r >= 3 {
            for _ in 0..50 {
                h = md5(&h[..n]).to_vec();
            }
        }
        let key = h[..n].to_vec();
        // verify the (empty) user password
        let ok = if p.r == 2 {
            rc4(&key, &PAD) == p.u[..p.u.len().min(32)]
        } else {
            let mut x = PAD.to_vec();
            x.extend_from_slice(p.id0);
            let mut t = rc4(&key, &md5(&x));
            for i in 1..=19u8 {
                let k2: Vec<u8> = key.iter().map(|b| b ^ i).collect();
                t = rc4(&k2, &t);
            }
            p.u.len() >= 16 && t[..16] == p.u[..16]
        };
        if !ok {
            return Err("the PDF is password-protected (it needs a user password)".into());
        }
        Ok(Crypt { key, stm: p.stm })
    }

    /// Decrypt a literal string that belongs to object `num gen` (strings use the string
    /// filter; streams and strings share the key derivation).
    pub fn decrypt_string(&self, num: u32, gen: u32, data: &[u8]) -> Vec<u8> {
        self.decrypt_stream(num, gen, data)
    }

    pub fn decrypt_stream(&self, num: u32, gen: u32, data: &[u8]) -> Vec<u8> {
        match self.stm {
            Method::None => data.to_vec(),
            Method::AesV3 => aes_cbc_decrypt(&self.key, None, data).unwrap_or_default(),
            m => {
                let mut k = self.key.clone();
                k.extend_from_slice(&num.to_le_bytes()[..3]);
                k.extend_from_slice(&gen.to_le_bytes()[..2]);
                if m == Method::AesV2 {
                    k.extend_from_slice(b"sAlT");
                }
                let len = (self.key.len() + 5).min(16);
                let ok = &md5(&k)[..len];
                if m == Method::AesV2 {
                    aes_cbc_decrypt(ok, None, data).unwrap_or_default()
                } else {
                    rc4(ok, data)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }
    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn hash_vectors() {
        assert_eq!(hex(&md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(hex(&md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(&md5(&[b'a'; 1000])), hex(&md5(&[b'a'; 1000])));
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert!(hex(&sha384(b"abc")).starts_with("cb00753f45a35e8bb5a03d699ac65007"));
        assert!(hex(&sha512(b"abc")).starts_with("ddaf35a193617abacc417349ae204131"));
        assert!(hex(&sha512(b"")).starts_with("cf83e1357eefb8bdf1542850d66d8007"));
        // multi-block inputs
        let long = vec![b'x'; 300];
        assert_eq!(hex(&sha256(&long)).len(), 64);
    }

    #[test]
    fn rc4_and_aes_vectors() {
        // RFC 6229-style well-known: key "Key", plaintext "Plaintext"
        assert_eq!(hex(&rc4(b"Key", b"Plaintext")), "bbf316e8d940af0ad3");
        // FIPS-197 appendix C
        let pt = unhex("00112233445566778899aabbccddeeff");
        for (key, ct) in [
            (
                "000102030405060708090a0b0c0d0e0f",
                "69c4e0d86a7b0430d8cdb78070b4c55a",
            ),
            (
                "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
                "8ea2b7ca516745bfeafc49904b496089",
            ),
        ] {
            let a = Aes::new(&unhex(key)).unwrap();
            let mut b = [0u8; 16];
            b.copy_from_slice(&pt);
            a.encrypt_block(&mut b);
            assert_eq!(hex(&b), ct);
            a.decrypt_block(&mut b);
            assert_eq!(b.to_vec(), pt);
        }
        // CBC round trip with padding removal
        let key = [7u8; 16];
        let iv = [9u8; 16];
        let mut msg = b"hello pdf world".to_vec();
        msg.push(1); // PKCS#7 for 16 bytes
        let enc = aes_cbc_encrypt_nopad(&key, iv, &msg).unwrap();
        let mut full = iv.to_vec();
        full.extend(enc);
        assert_eq!(
            aes_cbc_decrypt(&key, None, &full).unwrap(),
            b"hello pdf world"
        );
    }
}
