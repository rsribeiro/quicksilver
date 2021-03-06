//! A sound API that allows playing clips at given volumes
//!
//! On the desktop, currently all sounds are loaded into memory, but streaming sounds may be
//! introduced in the future. On the web, it can be different from browser to browser

use crate::{
    Result,
    error::QuicksilverError,
};
use futures::{Future, future};
use std::{
    error::Error,
    fmt,
    io::Error as IOError,
    path::Path
};
#[cfg(not(target_arch="wasm32"))]
use {
    rodio::{
        self,
        Sink,
        decoder::{Decoder, DecoderError},
        source::{SamplesConverter, Source, Amplify},
    },
    std::{
        fs::File,
        io::{Cursor, Read},
        sync::Arc
    }
};
#[cfg(target_arch="wasm32")]
use {
    futures::Async,
    std::io::ErrorKind,
    stdweb::{
        unstable::TryInto,
        Value
    }
};


/// A clip of sound, which may be streamed from disc or stored in memory
///
/// It can be played an arbitrary amount of times and concurrently with itself, meaning you don't
/// need more than one instance of a clip. However, if you want different clips with different
/// volumes, you can clone the Sound.
#[derive(Clone, Debug)]
pub struct Sound {
    #[cfg(not(target_arch="wasm32"))]
    val: Arc<Vec<u8>>,
    #[cfg(target_arch="wasm32")]
    sound: Value,
    volume: f32,
    loop_sound: bool
}


#[cfg(target_arch="wasm32")]
fn wasm_sound_error(error: &str) -> QuicksilverError {
    let error = IOError::new(ErrorKind::NotFound, error);
    let error: SoundError = error.into();
    error.into()
}

impl Sound {
    /// Start loading a sound from a given path
    pub fn load(path: impl AsRef<Path>) -> impl Future<Item = Sound, Error = QuicksilverError> {
        Sound::load_impl(path.as_ref())
    }

    #[cfg(not(target_arch="wasm32"))]
    fn load_impl(path: &Path) -> impl Future<Item = Sound, Error = QuicksilverError> {
        future::result(load(path))
    }

    #[cfg(target_arch="wasm32")]
    fn load_impl(path: &Path) -> impl Future<Item = Sound, Error = QuicksilverError> {
        let sound = js! {
            const audio = new Audio(@{path.to_str().expect("Path must be stringifiable")});
            audio.hasError = false;
            audio.onerror = (error) => audio.hasError = true;
            return audio;
        };
        future::poll_fn(move || {
            let error = js! ( return @{&sound}.hasError ).try_into();
            let ready = js! ( return @{&sound}.readyState ).try_into();
            match (error, ready) {
                (Ok(false), Ok(4)) => Ok(Async::Ready(Sound {
                    sound: sound.clone(),
                    volume: 1f32,
                    loop_sound: false
                })),
                (Ok(true), _) => Err(wasm_sound_error("Sound file not found or could not load")),
                (Ok(false), Ok(_)) => Ok(Async::NotReady),
                (Err(_), _) => Err(wasm_sound_error("Checking sound network state failed")),
                (_, Err(_)) => Err(wasm_sound_error("Checking sound ready state failed")),
            }
        })
    }
    

    /// Get the volume of the sound clip instance
    ///
    /// The volume is multiplicative, meaing 1 is the identity, 0 is silent, 2 is twice the
    /// amplitude, etc. Note that sound is not perceived linearly so results may not correspond as
    /// expected.
    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// Set the volume of the sound clip instance
    ///
    /// The volume is multiplicative, meaing 1 is the identity, 0 is silent, 2 is twice the
    /// amplitude, etc. Note that sound is not perceived linearly so results may not correspond as
    /// expected.
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
    }

    /// Set looping sound
    /// 
    /// If set sound will replay after it is finished.
    pub fn set_loop_sound(&mut self, loop_sound: bool) {
        self.loop_sound = loop_sound;
    }

    #[cfg(not(target_arch="wasm32"))]
    fn get_source(&self) -> Result<SamplesConverter<Amplify<Decoder<Cursor<Sound>>>, f32>> {
        Ok(Decoder::new(Cursor::new(self.clone()))?.amplify(self.volume).convert_samples())
    }

    /// Play the sound clip at its current volume
    ///
    /// The sound clip can be played over itself.
    ///
    /// Future changes in volume will not change the sound emitted by this method.
    pub fn play(&self) -> Result<StopHandle> {
        #[cfg(not(target_arch="wasm32"))] {
            let device = match rodio::default_output_device() {
                Some(device) => device,
                None => return Err(SoundError::NoOutputAvailable.into())
            };
            let sink = Sink::new(&device);
            if self.loop_sound {
                sink.append(self.get_source()?.repeat_infinite());
            } else {
                sink.append(self.get_source()?);
            }            
            StopHandle::new(sink)
        }
        #[cfg(target_arch="wasm32")] {
            let sound: Value = js! {
                let snd = @{&self.sound}.cloneNode();
                snd.loop = @{self.loop_sound};
                snd.play();
                return snd;
            };
            StopHandle::new(sound)
        }
    }
    
    #[cfg(not(target_arch="wasm32"))]
    //Play a silent sound so rodio startup doesn't interfere with application
    //Unfortunately this means even apps that don't use sound eat the startup penalty but it's not a
    //huge one
    pub(crate) fn initialize() {
        if let Some(ref device) = rodio::default_output_device() {
            rodio::play_raw(device, rodio::source::Empty::new())
        }
    }
}

#[cfg(not(target_arch="wasm32"))]
fn load(path: &Path) -> Result<Sound> {
    let mut bytes = Vec::new();
    File::open(path)?.read_to_end(&mut bytes)?;
    let val = Arc::new(bytes);
    let sound = Sound {
        val,
        volume: 1f32,
        loop_sound: false
    };
    Decoder::new(Cursor::new(sound.clone()))?;
    Ok(sound)
}

#[doc(hidden)]
#[cfg(not(target_arch="wasm32"))]
impl AsRef<[u8]> for Sound {
    fn as_ref(&self) -> &[u8] {
        self.val.as_ref().as_ref()
    }
}

#[derive(Debug)]
/// An error generated when loading a sound
pub enum SoundError {
    /// The sound file is not in an format that can be played
    UnrecognizedFormat,
    /// No output device was found to play the sound
    NoOutputAvailable,
    /// The Sound was not found or could not be loaded
    IOError(IOError)
}

impl fmt::Display for SoundError  {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.description())
    }
}

impl Error for SoundError {
    fn description(&self) -> &str {
        match self {
            SoundError::UnrecognizedFormat => "The sound file format was not recognized",
            SoundError::NoOutputAvailable => "There was no output device available for playing",
            SoundError::IOError(err) => err.description()
        }
    }

    fn cause(&self) -> Option<&dyn Error> {
        match self {
            SoundError::UnrecognizedFormat
                | SoundError::NoOutputAvailable => None,
            SoundError::IOError(err) => Some(err)
        }
    }

}

#[doc(hidden)]
#[cfg(not(target_arch="wasm32"))]
impl From<DecoderError> for SoundError {
    fn from(err: DecoderError) -> SoundError {
        match err {
            DecoderError::UnrecognizedFormat => SoundError::UnrecognizedFormat
        }
    }
}

#[doc(hidden)]
impl From<IOError> for SoundError {
    fn from(err: IOError) -> SoundError {
        SoundError::IOError(err)
    }
}

/// Stop handle
pub struct StopHandle {
    #[cfg(not(target_arch="wasm32"))]
    sink: Sink,
    #[cfg(target_arch="wasm32")]
    sound: Value,
}

impl StopHandle {
    #[cfg(not(target_arch="wasm32"))]
    fn new(sink: Sink) -> Result<StopHandle> {
        Ok(StopHandle{sink})
    }

    #[cfg(target_arch="wasm32")]
    fn new(sound: Value) -> Result<StopHandle> {
        Ok(StopHandle{sound})
    }

    /// stops the sound
    pub fn stop(self) -> Result<()> {
        #[cfg(not(target_arch="wasm32"))] {
            self.sink.stop();
        }
        #[cfg(target_arch="wasm32")] js! {
            @{&self.sound}.pause();
            @{&self.sound}.currentTime = 0;
        }
        Ok(())
    }
}

