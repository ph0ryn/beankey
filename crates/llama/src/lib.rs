use std::error::Error;
use std::ffi::{CStr, CString, c_char, c_float, c_int, c_void};
use std::fmt;
use std::path::Path;
use std::ptr::NonNull;

const ERROR_CAPACITY: usize = 256;
const CONTEXT_TOKEN_LIMIT: usize = 512;

unsafe extern "C" {
    fn beankey_llama_load(
        path: *const c_char,
        backend_directory: *const c_char,
        thread_count: c_int,
        error: *mut c_char,
        error_capacity: usize,
    ) -> *mut c_void;
    fn beankey_llama_free(handle: *mut c_void);
    fn beankey_llama_vocab_size(handle: *const c_void) -> c_int;
    fn beankey_llama_eos_token(handle: *const c_void) -> c_int;
    fn beankey_llama_tokenize(
        handle: *const c_void,
        text: *const c_char,
        text_length: c_int,
        tokens: *mut c_int,
        token_capacity: c_int,
        add_special: bool,
    ) -> c_int;
    fn beankey_llama_token_to_piece(
        handle: *const c_void,
        token: c_int,
        buffer: *mut c_char,
        buffer_capacity: c_int,
    ) -> c_int;
    fn beankey_llama_next_logits(
        handle: *mut c_void,
        tokens: *const c_int,
        token_count: c_int,
        logits: *mut c_float,
        logits_capacity: c_int,
    ) -> c_int;
}

#[derive(Debug, Eq, PartialEq)]
pub enum LlamaError {
    InvalidPath,
    Load(String),
    InputTooLong(usize),
    Tokenization,
    TokenPiece(c_int),
    Decode(c_int),
}

impl fmt::Display for LlamaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath => write!(formatter, "model path contains a null byte"),
            Self::Load(message) => write!(formatter, "model loading failed: {message}"),
            Self::InputTooLong(count) => {
                write!(
                    formatter,
                    "inference requires {count} tokens, exceeding 512"
                )
            }
            Self::Tokenization => write!(formatter, "llama.cpp tokenization failed"),
            Self::TokenPiece(token) => write!(formatter, "could not decode token {token}"),
            Self::Decode(code) => write!(formatter, "llama.cpp decode failed with code {code}"),
        }
    }
}

impl Error for LlamaError {}

pub struct LlamaContext {
    handle: NonNull<c_void>,
    vocabulary_size: usize,
}

// SAFETY: LlamaContext uniquely owns its native handle. Moving that ownership to another thread is
// safe; its mutable inference methods still require exclusive Rust access.
unsafe impl Send for LlamaContext {}

impl LlamaContext {
    pub fn load(
        path: impl AsRef<Path>,
        backend_directory: impl AsRef<Path>,
    ) -> Result<Self, LlamaError> {
        let path = CString::new(path.as_ref().as_os_str().as_encoded_bytes())
            .map_err(|_| LlamaError::InvalidPath)?;
        let backend_directory =
            CString::new(backend_directory.as_ref().as_os_str().as_encoded_bytes())
                .map_err(|_| LlamaError::InvalidPath)?;
        let threads = std::thread::available_parallelism()
            .map_or(1, std::num::NonZeroUsize::get)
            .min(c_int::MAX as usize) as c_int;
        let mut error = [0_i8; ERROR_CAPACITY];
        // SAFETY: The path and writable error buffer remain valid for this call.
        let handle = unsafe {
            beankey_llama_load(
                path.as_ptr(),
                backend_directory.as_ptr(),
                threads,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        let handle = NonNull::new(handle).ok_or_else(|| {
            // SAFETY: The C shim always terminates the supplied error buffer.
            let message = unsafe { CStr::from_ptr(error.as_ptr()) }
                .to_string_lossy()
                .into_owned();
            LlamaError::Load(message)
        })?;
        // SAFETY: A successfully constructed handle owns a valid vocabulary.
        let vocabulary_size = unsafe { beankey_llama_vocab_size(handle.as_ptr()) };
        Ok(Self {
            handle,
            vocabulary_size: usize::try_from(vocabulary_size)
                .map_err(|_| LlamaError::Load("invalid vocabulary size".into()))?,
        })
    }

    pub fn vocabulary_size(&self) -> usize {
        self.vocabulary_size
    }

    pub fn eos_token(&self) -> i32 {
        // SAFETY: The handle remains valid for the lifetime of self.
        unsafe { beankey_llama_eos_token(self.handle.as_ptr()) }
    }

    pub fn tokenize(&self, text: &str, add_special: bool) -> Result<Vec<i32>, LlamaError> {
        let text_length = c_int::try_from(text.len()).map_err(|_| LlamaError::Tokenization)?;
        let initial_capacity = text.len().saturating_add(2).max(1);
        let mut tokens = vec![0; initial_capacity];
        // SAFETY: All buffers remain valid and their exact lengths are supplied.
        let mut count = unsafe {
            beankey_llama_tokenize(
                self.handle.as_ptr(),
                text.as_ptr().cast(),
                text_length,
                tokens.as_mut_ptr(),
                c_int::try_from(tokens.len()).map_err(|_| LlamaError::Tokenization)?,
                add_special,
            )
        };
        if count < 0 {
            let required = count.checked_neg().ok_or(LlamaError::Tokenization)?;
            tokens.resize(
                usize::try_from(required).map_err(|_| LlamaError::Tokenization)?,
                0,
            );
            // SAFETY: The resized token buffer has the capacity requested by llama.cpp.
            count = unsafe {
                beankey_llama_tokenize(
                    self.handle.as_ptr(),
                    text.as_ptr().cast(),
                    text_length,
                    tokens.as_mut_ptr(),
                    required,
                    add_special,
                )
            };
        }
        let count = usize::try_from(count).map_err(|_| LlamaError::Tokenization)?;
        tokens.truncate(count);
        Ok(tokens)
    }

    pub fn token_to_piece(&self, token: i32) -> Result<Vec<u8>, LlamaError> {
        let mut buffer = vec![0_i8; 8];
        // SAFETY: The writable buffer remains valid for this call.
        let mut count = unsafe {
            beankey_llama_token_to_piece(
                self.handle.as_ptr(),
                token,
                buffer.as_mut_ptr(),
                c_int::try_from(buffer.len()).expect("small fixed buffer"),
            )
        };
        if count < 0 {
            let required = count.checked_neg().ok_or(LlamaError::TokenPiece(token))?;
            buffer.resize(
                usize::try_from(required).map_err(|_| LlamaError::TokenPiece(token))?,
                0,
            );
            // SAFETY: The resized piece buffer has the capacity requested by llama.cpp.
            count = unsafe {
                beankey_llama_token_to_piece(
                    self.handle.as_ptr(),
                    token,
                    buffer.as_mut_ptr(),
                    required,
                )
            };
        }
        let count = usize::try_from(count).map_err(|_| LlamaError::TokenPiece(token))?;
        Ok(buffer[..count]
            .iter()
            .map(|byte| byte.to_ne_bytes()[0])
            .collect())
    }

    pub fn next_logits(&mut self, tokens: &[i32]) -> Result<Vec<f32>, LlamaError> {
        if tokens.len() > CONTEXT_TOKEN_LIMIT {
            return Err(LlamaError::InputTooLong(tokens.len()));
        }
        let mut logits = vec![0.0; self.vocabulary_size];
        // SAFETY: Token and logit slices remain valid and exact lengths are supplied.
        let result = unsafe {
            beankey_llama_next_logits(
                self.handle.as_ptr(),
                tokens.as_ptr(),
                c_int::try_from(tokens.len()).expect("token limit fits c_int"),
                logits.as_mut_ptr(),
                c_int::try_from(logits.len()).map_err(|_| LlamaError::Decode(-1))?,
            )
        };
        if result != 0 {
            return Err(LlamaError::Decode(result));
        }
        Ok(logits)
    }
}

impl Drop for LlamaContext {
    fn drop(&mut self) {
        // SAFETY: This handle is uniquely owned and freed exactly once here.
        unsafe { beankey_llama_free(self.handle.as_ptr()) };
    }
}
