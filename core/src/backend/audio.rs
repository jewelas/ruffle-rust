use generational_arena::{Arena, Index};

pub mod decoders;
pub mod swf {
    pub use swf::{
        read, AudioCompression, CharacterId, Sound, SoundEnvelope, SoundEnvelopePoint, SoundEvent,
        SoundFormat, SoundInfo, SoundStreamHead,
    };
}

pub type AudioStreamHandle = Index;
pub type SoundHandle = Index;
pub type SoundInstanceHandle = Index;

type Error = Box<dyn std::error::Error>;

pub trait AudioBackend {
    fn play(&mut self);
    fn pause(&mut self);
    fn register_sound(&mut self, swf_sound: &swf::Sound) -> Result<SoundHandle, Error>;
    fn preload_sound_stream_head(
        &mut self,
        _clip_id: swf::CharacterId,
        _clip_frame: u16,
        _stream_info: &swf::SoundStreamHead,
    ) {
    }
    fn preload_sound_stream_block(
        &mut self,
        _clip_id: swf::CharacterId,
        _clip_frame: u16,
        _audio_data: &[u8],
    ) {
    }
    fn preload_sound_stream_end(&mut self, _clip_id: swf::CharacterId) {}

    /// Starts playing a sound instance that is not tied to a MovieClip timeline.
    /// In Flash, this is known as an "Event" sound.
    fn start_sound(
        &mut self,
        sound: SoundHandle,
        settings: &swf::SoundInfo,
    ) -> Result<SoundInstanceHandle, Error>;

    fn start_stream(
        &mut self,
        clip_id: crate::prelude::CharacterId,
        clip_frame: u16,
        clip_data: crate::tag_utils::SwfSlice,
        handle: &swf::SoundStreamHead,
    ) -> Result<AudioStreamHandle, Error>;

    /// Stops a playing sound instance.
    /// No-op if the sound is not playing.
    fn stop_sound(&mut self, sound: SoundInstanceHandle);

    /// Stops a playing stream souund.
    /// Should be called whenever a MovieClip timeline stops playing or seeks to a new frame.
    fn stop_stream(&mut self, stream: AudioStreamHandle);

    /// Good ol' stopAllSounds() :-)
    fn stop_all_sounds(&mut self);

    /// Stops all active sound instances of a particular sound.
    /// Used by SWF `StartSound` tag with `SoundEvent::Stop`.
    fn stop_sounds_with_handle(&mut self, handle: SoundHandle);

    /// Returns whether a sound clip is playing.
    /// Used by SWF `StartSouynd` tag with `SoundEvent:Start`,
    /// which only plays a sound if that sound is not already playing.
    fn is_sound_playing_with_handle(&mut self, handle: SoundHandle) -> bool;

    /// Get the duration of a sound in milliseconds.
    /// Returns `None` if sound is not registered.
    fn get_sound_duration(&self, sound: SoundHandle) -> Option<u32>;

    // TODO: Eventually remove this/move it to library.
    fn is_loading_complete(&self) -> bool {
        true
    }
    fn tick(&mut self) {}

    /// Inform the audio backend of the current stage frame rate.
    ///
    /// This is only necessary if your particular audio backend needs to know
    /// what the stage frame rate is. Otherwise, you are free to avoid
    /// implementing it.
    fn set_frame_rate(&mut self, _frame_rate: f64) {}
}

/// Audio backend that ignores all audio.
pub struct NullAudioBackend {
    sounds: Arena<()>,
    streams: Arena<()>,
}

impl NullAudioBackend {
    pub fn new() -> NullAudioBackend {
        NullAudioBackend {
            streams: Arena::new(),
            sounds: Arena::new(),
        }
    }
}

impl AudioBackend for NullAudioBackend {
    fn play(&mut self) {}
    fn pause(&mut self) {}
    fn register_sound(&mut self, _sound: &swf::Sound) -> Result<SoundHandle, Error> {
        Ok(self.sounds.insert(()))
    }

    fn start_sound(
        &mut self,
        _sound: SoundHandle,
        _sound_info: &swf::SoundInfo,
    ) -> Result<SoundInstanceHandle, Error> {
        Ok(SoundInstanceHandle::from_raw_parts(0, 0))
    }

    fn start_stream(
        &mut self,
        _clip_id: crate::prelude::CharacterId,
        _stream_start_frame: u16,
        _clip_data: crate::tag_utils::SwfSlice,
        _handle: &swf::SoundStreamHead,
    ) -> Result<AudioStreamHandle, Error> {
        Ok(self.streams.insert(()))
    }

    fn stop_sound(&mut self, _sound: SoundInstanceHandle) {}

    fn stop_stream(&mut self, stream: AudioStreamHandle) {
        self.streams.remove(stream);
    }
    fn stop_all_sounds(&mut self) {}
    fn stop_sounds_with_handle(&mut self, _handle: SoundHandle) {}
    fn is_sound_playing_with_handle(&mut self, _handle: SoundHandle) -> bool {
        false
    }

    fn get_sound_duration(&self, _sound: SoundHandle) -> Option<u32> {
        None
    }
}

impl Default for NullAudioBackend {
    fn default() -> Self {
        NullAudioBackend::new()
    }
}
