//! The crypto functions moonlight-common-c asks its host program for, so no
//! OpenSSL is shipped. Behaviour follows the core's own OpenSSL version:
//! AES-128-GCM per message (12 or 16 byte IV, no extra data) and AES-128-CBC
//! whose chaining carries over between calls unless the IV is reset.
use aes::cipher::{generic_array::GenericArray, BlockDecrypt, BlockEncrypt, KeyInit};
use aes::Aes128;
use aes_gcm::{aead::AeadInPlace, aes::cipher::consts::{U12, U16}, AesGcm};
use std::os::raw::{c_int, c_uchar};

const AES_CBC: c_int = 1;
const AES_GCM: c_int = 2;
const RESET_IV: c_int = 0x01;
const FINISH: c_int = 0x02;
const PAD: c_int = 0x04;

pub struct Context {
    chain: Option<[u8; 16]>,
}

#[no_mangle]
pub extern "C" fn PltCreateCryptoContext() -> *mut Context {
    Box::into_raw(Box::new(Context { chain: None }))
}

#[no_mangle]
pub unsafe extern "C" fn PltDestroyCryptoContext(ctx: *mut Context) {
    if !ctx.is_null() {
        drop(Box::from_raw(ctx));
    }
}

#[no_mangle]
pub unsafe extern "C" fn PltGenerateRandomData(data: *mut c_uchar, length: c_int) {
    if length > 0 {
        let _ = getrandom::getrandom(std::slice::from_raw_parts_mut(data, length as usize));
    }
}

unsafe fn key(key: *const c_uchar, length: c_int) -> Option<Aes128> {
    (length == 16).then(|| Aes128::new(GenericArray::from_slice(std::slice::from_raw_parts(key, 16))))
}

unsafe fn gcm(encrypt: bool, k: *const c_uchar, iv: &[u8], tag: *mut c_uchar, tag_length: c_int, data: &mut [u8]) -> bool {
    if tag_length != 16 {
        return false;
    }
    let key_bytes = GenericArray::from_slice(std::slice::from_raw_parts(k, 16));
    let tag = std::slice::from_raw_parts_mut(tag, 16);
    macro_rules! run {
        ($size:ty) => {{
            let cipher = AesGcm::<Aes128, $size>::new(key_bytes);
            let nonce = GenericArray::from_slice(iv);
            if encrypt {
                match cipher.encrypt_in_place_detached(nonce, &[], data) {
                    Ok(made) => {
                        tag.copy_from_slice(&made);
                        true
                    }
                    Err(_) => false,
                }
            } else {
                cipher.decrypt_in_place_detached(nonce, &[], data, GenericArray::from_slice(tag)).is_ok()
            }
        }};
    }
    match iv.len() {
        12 => run!(U12),
        16 => run!(U16),
        _ => false,
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn message(
    encrypt: bool, ctx: *mut Context, algorithm: c_int, flags: c_int, k: *const c_uchar, key_length: c_int,
    iv: *const c_uchar, iv_length: c_int, tag: *mut c_uchar, tag_length: c_int, input: *mut c_uchar,
    input_length: c_int, output: *mut c_uchar, output_length: *mut c_int,
) -> bool {
    let (Some(ctx), true) = (ctx.as_mut(), input_length >= 0 && key_length == 16) else { return false };
    let iv = std::slice::from_raw_parts(iv, iv_length.max(0) as usize);
    let mut length = input_length as usize;
    if algorithm == AES_GCM {
        let mut data = std::slice::from_raw_parts(input, length).to_vec();
        if !gcm(encrypt, k, iv, tag, tag_length, &mut data) {
            return false;
        }
        std::ptr::copy_nonoverlapping(data.as_ptr(), output, length);
        *output_length = length as c_int;
        return true;
    }
    if algorithm != AES_CBC || iv.len() != 16 {
        return false;
    }
    let Some(cipher) = key(k, key_length) else { return false };
    if ctx.chain.is_none() || flags & RESET_IV != 0 {
        ctx.chain = Some(iv.try_into().unwrap());
    }
    if encrypt && flags & PAD != 0 {
        // The caller left room for the padding, as the core's OpenSSL version requires.
        let padded = length.div_ceil(16) * 16;
        let fill = (16 - length % 16) as u8;
        for at in length..padded {
            *input.add(at) = fill;
        }
        length = padded;
    }
    if length % 16 != 0 {
        return false;
    }
    let data = std::slice::from_raw_parts(input, length).to_vec();
    let out = std::slice::from_raw_parts_mut(output, length);
    let mut chain = ctx.chain.unwrap();
    for (block, place) in data.chunks(16).zip(out.chunks_mut(16)) {
        if encrypt {
            let mut b = [0u8; 16];
            for i in 0..16 {
                b[i] = block[i] ^ chain[i];
            }
            let mut g = GenericArray::from(b);
            cipher.encrypt_block(&mut g);
            place.copy_from_slice(&g);
            chain.copy_from_slice(&g);
        } else {
            let mut g = GenericArray::clone_from_slice(block);
            cipher.decrypt_block(&mut g);
            for i in 0..16 {
                place[i] = g[i] ^ chain[i];
            }
            chain.copy_from_slice(block);
        }
    }
    ctx.chain = Some(chain);
    if !encrypt && flags & FINISH != 0 {
        // Remove PKCS#7 padding, as EVP_DecryptFinal_ex does.
        let pad = *out.last().unwrap_or(&0) as usize;
        if pad == 0 || pad > 16 || pad > length || out[length - pad..].iter().any(|&b| b as usize != pad) {
            return false;
        }
        length -= pad;
    }
    *output_length = length as c_int;
    true
}

#[no_mangle]
pub unsafe extern "C" fn PltEncryptMessage(
    ctx: *mut Context, algorithm: c_int, flags: c_int, k: *const c_uchar, key_length: c_int, iv: *const c_uchar,
    iv_length: c_int, tag: *mut c_uchar, tag_length: c_int, input: *mut c_uchar, input_length: c_int,
    output: *mut c_uchar, output_length: *mut c_int,
) -> bool {
    message(true, ctx, algorithm, flags, k, key_length, iv, iv_length, tag, tag_length, input, input_length, output, output_length)
}

#[no_mangle]
pub unsafe extern "C" fn PltDecryptMessage(
    ctx: *mut Context, algorithm: c_int, flags: c_int, k: *const c_uchar, key_length: c_int, iv: *const c_uchar,
    iv_length: c_int, tag: *mut c_uchar, tag_length: c_int, input: *mut c_uchar, input_length: c_int,
    output: *mut c_uchar, output_length: *mut c_int,
) -> bool {
    message(false, ctx, algorithm, flags, k, key_length, iv, iv_length, tag, tag_length, input, input_length, output, output_length)
}
