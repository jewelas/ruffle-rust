use crate::avm1::movie_clip::create_movie_object;
use crate::avm1::object::{Object, TYPE_OF_MOVIE_CLIP};
use crate::avm1::Value;
use crate::backend::audio::AudioStreamHandle;
use crate::character::Character;
use crate::display_object::{DisplayObject, DisplayObjectBase};
use crate::font::Font;
use crate::graphic::Graphic;
use crate::matrix::Matrix;
use crate::morph_shape::MorphShapeStatic;
use crate::player::{ActionQueue, RenderContext, UpdateContext};
use crate::prelude::*;
use crate::tag_utils::{self, DecodeResult, SwfStream};
use crate::text::Text;
use gc_arena::{Gc, GcCell, MutationContext};
use std::collections::{BTreeMap, HashMap};
use swf::read::SwfRead;

type Depth = i16;
type FrameNumber = u16;

#[derive(Clone, Debug)]
pub struct MovieClip<'gc> {
    base: DisplayObjectBase<'gc>,
    swf_version: u8,
    static_data: Gc<'gc, MovieClipStatic>,
    tag_stream_pos: u64,
    is_playing: bool,
    current_frame: FrameNumber,
    audio_stream: Option<AudioStreamHandle>,
    children: BTreeMap<Depth, DisplayNode<'gc>>,
    object: GcCell<'gc, Object<'gc>>,
}

impl<'gc> MovieClip<'gc> {
    pub fn new(swf_version: u8, gc_context: MutationContext<'gc, '_>) -> Self {
        Self {
            base: Default::default(),
            swf_version,
            static_data: Gc::allocate(gc_context, MovieClipStatic::default()),
            tag_stream_pos: 0,
            is_playing: false,
            current_frame: 0,
            audio_stream: None,
            children: BTreeMap::new(),
            object: GcCell::allocate(gc_context, create_movie_object(gc_context)),
        }
    }

    pub fn new_with_data(
        swf_version: u8,
        gc_context: MutationContext<'gc, '_>,
        id: CharacterId,
        tag_stream_start: u64,
        tag_stream_len: usize,
        num_frames: u16,
    ) -> Self {
        Self {
            base: Default::default(),
            swf_version,
            static_data: Gc::allocate(
                gc_context,
                MovieClipStatic {
                    id,
                    tag_stream_start,
                    tag_stream_len,
                    total_frames: num_frames,
                    audio_stream_info: None,
                    frame_labels: HashMap::new(),
                },
            ),
            tag_stream_pos: 0,
            is_playing: true,
            current_frame: 0,
            audio_stream: None,
            children: BTreeMap::new(),
            object: GcCell::allocate(gc_context, create_movie_object(gc_context)),
        }
    }

    pub fn playing(&self) -> bool {
        self.is_playing
    }

    pub fn next_frame(&mut self, self_cell: DisplayNode<'gc>, action_queue: &mut ActionQueue<'gc>) {
        if self.current_frame() < self.total_frames() {
            self.goto_frame(self_cell, action_queue, self.current_frame + 1, true);
        }
    }

    pub fn play(&mut self) {
        // Can only play clips with multiple frames.
        if self.total_frames() > 1 {
            self.is_playing = true;
        }
    }

    pub fn prev_frame(&mut self, self_cell: DisplayNode<'gc>, action_queue: &mut ActionQueue<'gc>) {
        if self.current_frame > 1 {
            self.goto_frame(self_cell, action_queue, self.current_frame - 1, true);
        }
    }

    pub fn stop(&mut self) {
        self.is_playing = false;
    }

    /// Queues up a goto to the specified frame.
    /// `frame` should be 1-based.
    pub fn goto_frame(
        &mut self,
        self_cell: DisplayNode<'gc>,
        action_queue: &mut ActionQueue<'gc>,
        frame: FrameNumber,
        stop: bool,
    ) {
        if frame != self.current_frame {
            action_queue.queue_goto(self_cell, frame);
        }

        if stop {
            self.stop();
        } else {
            self.play();
        }
    }

    pub fn x(&self) -> f32 {
        self.matrix().tx / Twips::TWIPS_PER_PIXEL as f32
    }

    pub fn set_x(&mut self, val: f32) {
        self.matrix_mut().tx = val * Twips::TWIPS_PER_PIXEL as f32;
    }

    pub fn y(&self) -> f32 {
        self.matrix().ty / Twips::TWIPS_PER_PIXEL as f32
    }

    pub fn set_y(&mut self, val: f32) {
        self.matrix_mut().ty = val * Twips::TWIPS_PER_PIXEL as f32;
    }

    pub fn x_scale(&self) -> f32 {
        self.matrix().a * 100.0
    }

    pub fn set_x_scale(&mut self, val: f32) {
        self.matrix_mut().a = val / 100.0;
    }

    pub fn y_scale(&self) -> f32 {
        self.matrix().d * 100.0
    }

    pub fn set_y_scale(&mut self, val: f32) {
        self.matrix_mut().d = val / 100.0;
    }

    pub fn current_frame(&self) -> FrameNumber {
        self.current_frame
    }

    pub fn total_frames(&self) -> FrameNumber {
        self.static_data.total_frames
    }

    pub fn rotation(&self) -> f32 {
        // TODO: Cache the user-friendly transform values like rotation.
        let matrix = self.matrix();
        f32::atan2(matrix.b, matrix.a).to_degrees()
    }

    pub fn set_rotation(&mut self, degrees: f32) {
        // TODO: Use cached user-friendly transform values.
        let angle = degrees.to_radians();
        let cos = f32::cos(angle);
        let sin = f32::sin(angle);
        let scale_x = self.x_scale() / 100.0;
        let scale_y = self.y_scale() / 100.0;

        let matrix = self.matrix_mut();
        *matrix = Matrix {
            a: scale_x * cos,
            b: scale_x * sin,
            c: scale_y * cos,
            d: -scale_y * sin,
            tx: matrix.tx,
            ty: matrix.ty,
        };
    }

    pub fn frames_loaded(&self) -> FrameNumber {
        // TODO(Herschel): root needs to progressively stream in frames.
        self.static_data.total_frames
    }

    pub fn get_child_by_name(&self, name: &str) -> Option<&DisplayNode<'gc>> {
        // TODO: Make a HashMap from name -> child?
        self.children
            .values()
            .find(|child| child.read().name() == name)
    }

    pub fn frame_label_to_number(&self, frame_label: &str) -> Option<FrameNumber> {
        self.static_data.frame_labels.get(frame_label).copied()
    }

    fn tag_stream_start(&self) -> u64 {
        self.static_data.tag_stream_start
    }

    fn tag_stream_len(&self) -> usize {
        self.static_data.tag_stream_len
    }

    fn reader<'a>(
        &self,
        context: &UpdateContext<'a, '_, '_>,
    ) -> swf::read::Reader<std::io::Cursor<&'a [u8]>> {
        let mut cursor = std::io::Cursor::new(
            &context.swf_data[self.tag_stream_start() as usize
                ..self.tag_stream_start() as usize + self.tag_stream_len()],
        );
        cursor.set_position(self.tag_stream_pos);
        swf::read::Reader::new(cursor, context.swf_version)
    }

    fn run_frame_internal(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        run_display_actions: bool,
    ) {
        // Advance frame number.
        if self.current_frame() < self.total_frames() {
            self.current_frame += 1;
        } else if self.total_frames() > 1 {
            // Looping acts exactly like a gotoAndPlay(1).
            // Specifically, object that existed on frame 1 should not be destroyed
            // and recreated.
            self.run_goto(context, 1);
            return;
        } else {
            // Single frame clips do not play.
            self.stop();
        }

        let _tag_pos = self.tag_stream_pos;
        let mut reader = self.reader(context);

        use swf::TagCode;

        let tag_callback = |reader: &mut _, tag_code, tag_len| match tag_code {
            TagCode::DoAction => self.do_action(context, reader, tag_len),
            TagCode::PlaceObject if run_display_actions => {
                self.place_object(context, reader, tag_len, 1)
            }
            TagCode::PlaceObject2 if run_display_actions => {
                self.place_object(context, reader, tag_len, 2)
            }
            TagCode::PlaceObject3 if run_display_actions => {
                self.place_object(context, reader, tag_len, 3)
            }
            TagCode::PlaceObject4 if run_display_actions => {
                self.place_object(context, reader, tag_len, 4)
            }
            TagCode::RemoveObject if run_display_actions => self.remove_object(context, reader, 1),
            TagCode::RemoveObject2 if run_display_actions => self.remove_object(context, reader, 2),
            TagCode::SetBackgroundColor => self.set_background_color(context, reader),
            TagCode::StartSound => self.start_sound_1(context, reader),
            TagCode::SoundStreamBlock => self.sound_stream_block(context, reader),
            _ => Ok(()),
        };
        let _ = tag_utils::decode_tags(&mut reader, tag_callback, TagCode::ShowFrame);

        self.tag_stream_pos = reader.get_ref().position();
    }

    fn instantiate_child(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        id: CharacterId,
        depth: Depth,
        copy_previous_properties: bool,
    ) -> Option<DisplayNode<'gc>> {
        if let Ok(child_cell) = context
            .library
            .instantiate_display_object(id, context.gc_context)
        {
            // Remove previous child from children list,
            // and add new childonto front of the list.
            let prev_child = self.children.insert(depth, child_cell);
            if let Some(prev_child) = prev_child {
                self.remove_child_from_exec_list(context.gc_context, prev_child);
            }
            self.add_child_to_exec_list(context.gc_context, child_cell);
            {
                let mut child = child_cell.write(context.gc_context);
                // Set initial properties for child.
                child.set_parent(Some(context.active_clip));
                child.set_place_frame(self.current_frame);
                if copy_previous_properties {
                    if let Some(prev_child) = prev_child {
                        child.copy_display_properties_from(prev_child);
                    }
                }
                let prev_clip = context.active_clip;
                // Run first frame.
                context.active_clip = child_cell;
                child.run_frame(context);
                context.active_clip = prev_clip;
            }
            Some(child_cell)
        } else {
            log::error!("Unable to instantiate display node id {}", id);
            None
        }
    }
    /// Adds a child to the front of the execution list.
    /// This does not affect the render list.
    fn add_child_to_exec_list(
        &mut self,
        gc_context: MutationContext<'gc, '_>,
        child_cell: DisplayNode<'gc>,
    ) {
        if let Some(head) = self.first_child() {
            head.write(gc_context).set_prev_sibling(Some(child_cell));
            child_cell.write(gc_context).set_next_sibling(Some(head));
        }
        self.set_first_child(Some(child_cell));
    }
    /// Removes a child from the execution list.
    /// This does not affect the render list.
    fn remove_child_from_exec_list(
        &mut self,
        gc_context: MutationContext<'gc, '_>,
        child_cell: DisplayNode<'gc>,
    ) {
        let mut child = child_cell.write(gc_context);
        // Remove from children linked list.
        let prev = child.prev_sibling();
        let next = child.next_sibling();
        if let Some(prev) = prev {
            prev.write(gc_context).set_next_sibling(next);
        }
        if let Some(next) = next {
            next.write(gc_context).set_prev_sibling(prev);
        }
        if let Some(head) = self.first_child() {
            if GcCell::ptr_eq(head, child_cell) {
                self.set_first_child(next);
            }
        }
        // Flag child as removed.
        child.set_removed(true);
    }
    pub fn run_goto(&mut self, context: &mut UpdateContext<'_, 'gc, '_>, frame: FrameNumber) {
        // Flash gotos are tricky:
        // 1) Conceptually, a goto should act like the playhead is advancing forward or
        //    backward to a frame.
        // 2) However, MovieClip timelines are stored as deltas from frame to frame,
        //    so for rewinds, we must restart to frame 1 and play forward.
        // 3) Objects that would persist over the goto conceptually should not be
        //    destroyed and recreated; they should keep their properties.
        //    Particularly for rewinds, the object should persist if it was created
        //      *before* the frame we are going to. (DisplayNode::place_frame).
        // 4) We want to avoid creating objects just to destroy them if they aren't on
        //    the goto frame, so we should instead aggregate the deltas into a final list
        //    of commands, and THEN modify the children as necessary.

        // This map will maintain a map of depth -> placement commands.
        // TODO: Move this to UpdateContext to avoid allocations.
        let mut goto_commands = fnv::FnvHashMap::default();

        let is_rewind = if frame < self.current_frame() {
            // Because we can only step forward, we have to start at frame 1
            // when rewinding.
            self.tag_stream_pos = 0;
            self.current_frame = 0;

            // Remove all display objects that were created after the desination frame.
            // TODO: We want to do something like self.children.retain here,
            // but BTreeMap::retain does not exist.
            let children: smallvec::SmallVec<[_; 16]> = self
                .children
                .iter()
                .filter_map(|(depth, clip)| {
                    if clip.read().place_frame() > frame {
                        Some((*depth, *clip))
                    } else {
                        None
                    }
                })
                .collect();
            for (depth, child) in children {
                self.children.remove(&depth);
                self.remove_child_from_exec_list(context.gc_context, child);
            }
            true
        } else {
            false
        };

        // Step through the intermediate frames, and aggregate the deltas of each frame.
        let mut frame_pos = self.tag_stream_pos;
        let mut reader = self.reader(context);
        let gc_context = context.gc_context;
        while self.current_frame < frame {
            self.current_frame += 1;
            frame_pos = reader.get_inner().position();

            use swf::TagCode;
            let tag_callback = |reader: &mut _, tag_code, tag_len| match tag_code {
                TagCode::PlaceObject => {
                    self.goto_place_object(reader, tag_len, 1, &mut goto_commands)
                }
                TagCode::PlaceObject2 => {
                    self.goto_place_object(reader, tag_len, 2, &mut goto_commands)
                }
                TagCode::PlaceObject3 => {
                    self.goto_place_object(reader, tag_len, 3, &mut goto_commands)
                }
                TagCode::PlaceObject4 => {
                    self.goto_place_object(reader, tag_len, 4, &mut goto_commands)
                }
                TagCode::RemoveObject => {
                    self.goto_remove_object(reader, 1, gc_context, &mut goto_commands, is_rewind)
                }
                TagCode::RemoveObject2 => {
                    self.goto_remove_object(reader, 2, gc_context, &mut goto_commands, is_rewind)
                }
                _ => Ok(()),
            };
            let _ = tag_utils::decode_tags(&mut reader, tag_callback, TagCode::ShowFrame);
        }

        // Run the list of goto commands to actually create and update the display objects.
        let run_goto_command =
            |clip: &mut MovieClip<'gc>,
             context: &mut UpdateContext<'_, 'gc, '_>,
             (&depth, params): (&Depth, &GotoPlaceObject)| {
                let (was_instantiated, child) = match clip.children.get_mut(&depth).copied() {
                    // For rewinds, if an object was created before the final frame,
                    // it will exist on the final frame as well. Re-use this object
                    // instead of recreating.
                    Some(prev_child) => (false, prev_child),
                    None => {
                        if let Some(child) = clip.instantiate_child(
                            context,
                            params.id(),
                            depth,
                            params.modifies_original_item(),
                        ) {
                            (true, child)
                        } else {
                            return;
                        }
                    }
                };

                // Apply final delta to display pamareters.
                let mut child = child.write(context.gc_context);
                child.apply_place_object(&params.place_object);
                if was_instantiated {
                    // Set the placement frame for the new object to the frame
                    // it is actually created on.
                    child.set_place_frame(params.frame);
                }
            };

        // We have to be sure that queued actions are generated in the same order
        // as if the playhead had reached this frame normally.
        // First, run frames for children that were created before this frame.
        goto_commands
            .iter()
            .filter(|(_, params)| params.frame < frame)
            .for_each(|goto| run_goto_command(self, context, goto));

        // Next, run the final frame for the parent clip.
        // Re-run the final frame without display tags (DoAction, StartSound, etc.)
        self.current_frame = frame - 1;
        self.tag_stream_pos = frame_pos;
        self.run_frame_internal(context, false);

        // Finally, run frames for children that are placed on this frame.
        goto_commands
            .iter()
            .filter(|(_, params)| params.frame >= frame)
            .for_each(|goto| run_goto_command(self, context, goto));
    }

    /// Handles a PlaceObject tag when running a goto action.
    #[inline]
    fn goto_place_object<'a>(
        &mut self,
        reader: &mut SwfStream<&'a [u8]>,
        tag_len: usize,
        version: u8,
        goto_commands: &mut fnv::FnvHashMap<Depth, GotoPlaceObject>,
    ) -> DecodeResult {
        let place_object = if version == 1 {
            reader.read_place_object(tag_len)
        } else {
            reader.read_place_object_2_or_3(version)
        }?;

        // We merge the deltas from this PlaceObject with the previous command.
        let depth = place_object.depth;
        let mut goto_place = GotoPlaceObject {
            frame: self.current_frame,
            place_object,
        };
        goto_commands
            .entry(depth)
            .and_modify(|prev_place| prev_place.merge(&mut goto_place))
            .or_insert(goto_place);

        Ok(())
    }

    /// Handle a RemoveObject tag when running a goto action.
    #[inline]
    fn goto_remove_object<'a>(
        &mut self,
        reader: &mut SwfStream<&'a [u8]>,
        version: u8,
        gc_context: MutationContext<'gc, '_>,
        goto_commands: &mut fnv::FnvHashMap<Depth, GotoPlaceObject>,
        is_rewind: bool,
    ) -> DecodeResult {
        let remove_object = if version == 1 {
            reader.read_remove_object_1()
        } else {
            reader.read_remove_object_2()
        }?;
        goto_commands.remove(&remove_object.depth);
        if !is_rewind {
            // For fast-forwards, if this tag were to remove an object
            // that existed before the goto, then we can remove that child right away.
            // Don't do this for rewinds, because they conceptually
            // start from an empty display list, and we also want to examine
            // the old children to decide if they persist (place_frame <= goto_frame).
            if let Some(child) = self.children.remove(&remove_object.depth) {
                self.remove_child_from_exec_list(gc_context, child);
            }
        }
        Ok(())
    }
}

impl<'gc> DisplayObject<'gc> for MovieClip<'gc> {
    impl_display_object!(base);

    fn id(&self) -> CharacterId {
        self.static_data.id
    }

    fn run_frame(&mut self, context: &mut UpdateContext<'_, 'gc, '_>) {
        // Children must run first.
        let prev_clip = context.active_clip;
        for child in self.children() {
            context.active_clip = child;
            child.write(context.gc_context).run_frame(context);
        }
        context.active_clip = prev_clip;

        // Run myself.
        if self.is_playing {
            self.run_frame_internal(context, true);
        }
    }

    fn render(&self, context: &mut RenderContext<'_, 'gc>) {
        context.transform_stack.push(self.transform());
        crate::display_object::render_children(context, &self.children);
        context.transform_stack.pop();
    }

    fn mouse_pick(
        &self,
        _self_node: DisplayNode<'gc>,
        point: (Twips, Twips),
    ) -> Option<DisplayNode<'gc>> {
        for child in self.children.values().rev() {
            let result = child.read().mouse_pick(*child, point);
            if result.is_some() {
                return result;
            }
        }

        None
    }

    fn as_movie_clip(&self) -> Option<&crate::movie_clip::MovieClip<'gc>> {
        Some(self)
    }

    fn as_movie_clip_mut(&mut self) -> Option<&mut crate::movie_clip::MovieClip<'gc>> {
        Some(self)
    }

    fn post_instantiation(
        &mut self,
        gc_context: MutationContext<'gc, '_>,
        display_object: DisplayNode<'gc>,
    ) {
        let mut object = self.object.write(gc_context);
        object.set_display_node(display_object);
        object.set_type_of(TYPE_OF_MOVIE_CLIP);
    }

    fn object(&self) -> Value<'gc> {
        Value::Object(self.object)
    }
}

unsafe impl<'gc> gc_arena::Collect for MovieClip<'gc> {
    #[inline]
    fn trace(&self, cc: gc_arena::CollectionContext) {
        for child in self.children.values() {
            child.trace(cc);
        }
        self.base.trace(cc);
        self.static_data.trace(cc);
        self.object.trace(cc);
    }
}

// Preloading of definition tags
impl<'gc, 'a> MovieClip<'gc> {
    pub fn preload(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        morph_shapes: &mut fnv::FnvHashMap<CharacterId, MorphShapeStatic>,
    ) {
        use swf::TagCode;
        // TODO: Re-creating static data because preload step occurs after construction.
        // Should be able to hoist this up somewhere, or use MaybeUninit.
        let mut static_data = (&*self.static_data).clone();
        let mut reader = self.reader(context);
        let mut cur_frame = 1;
        let mut ids = fnv::FnvHashMap::default();
        let tag_callback = |reader: &mut _, tag_code, tag_len| match tag_code {
            TagCode::DefineBits => self.define_bits(context, reader, tag_len),
            TagCode::DefineBitsJpeg2 => self.define_bits_jpeg_2(context, reader, tag_len),
            TagCode::DefineBitsJpeg3 => self.define_bits_jpeg_3(context, reader, tag_len),
            TagCode::DefineBitsJpeg4 => self.define_bits_jpeg_4(context, reader, tag_len),
            TagCode::DefineBitsLossless => self.define_bits_lossless(context, reader, 1),
            TagCode::DefineBitsLossless2 => self.define_bits_lossless(context, reader, 2),
            TagCode::DefineButton => self.define_button_1(context, reader),
            TagCode::DefineButton2 => self.define_button_2(context, reader),
            TagCode::DefineEditText => self.define_edit_text(context, reader),
            TagCode::DefineFont => self.define_font_1(context, reader),
            TagCode::DefineFont2 => self.define_font_2(context, reader),
            TagCode::DefineFont3 => self.define_font_3(context, reader),
            TagCode::DefineFont4 => unimplemented!(),
            TagCode::DefineMorphShape => self.define_morph_shape(context, reader, morph_shapes, 1),
            TagCode::DefineMorphShape2 => self.define_morph_shape(context, reader, morph_shapes, 2),
            TagCode::DefineShape => self.define_shape(context, reader, 1),
            TagCode::DefineShape2 => self.define_shape(context, reader, 2),
            TagCode::DefineShape3 => self.define_shape(context, reader, 3),
            TagCode::DefineShape4 => self.define_shape(context, reader, 4),
            TagCode::DefineSound => self.define_sound(context, reader, tag_len),
            TagCode::DefineSprite => self.define_sprite(context, reader, tag_len, morph_shapes),
            TagCode::DefineText => self.define_text(context, reader, 1),
            TagCode::DefineText2 => self.define_text(context, reader, 2),
            TagCode::FrameLabel => {
                self.frame_label(context, reader, tag_len, cur_frame, &mut static_data)
            }
            TagCode::JpegTables => self.jpeg_tables(context, reader, tag_len),
            TagCode::PlaceObject => {
                self.preload_place_object(context, reader, tag_len, &mut ids, morph_shapes, 1)
            }
            TagCode::PlaceObject2 => {
                self.preload_place_object(context, reader, tag_len, &mut ids, morph_shapes, 2)
            }
            TagCode::PlaceObject3 => {
                self.preload_place_object(context, reader, tag_len, &mut ids, morph_shapes, 3)
            }
            TagCode::PlaceObject4 => {
                self.preload_place_object(context, reader, tag_len, &mut ids, morph_shapes, 4)
            }
            TagCode::RemoveObject => self.preload_remove_object(context, reader, &mut ids, 1),
            TagCode::RemoveObject2 => self.preload_remove_object(context, reader, &mut ids, 2),
            TagCode::ShowFrame => self.preload_show_frame(context, reader, &mut cur_frame),
            TagCode::SoundStreamHead => {
                self.preload_sound_stream_head(context, reader, &mut static_data, 1)
            }
            TagCode::SoundStreamHead2 => {
                self.preload_sound_stream_head(context, reader, &mut static_data, 2)
            }
            TagCode::SoundStreamBlock => {
                self.preload_sound_stream_block(context, reader, &mut static_data, tag_len)
            }
            _ => Ok(()),
        };
        let _ = tag_utils::decode_tags(&mut reader, tag_callback, TagCode::End);
        self.static_data = Gc::allocate(context.gc_context, static_data);

        // Finalize audio stream.
        if self.static_data.audio_stream_info.is_some() {
            context.audio.preload_sound_stream_end(self.id());
        }
    }

    #[inline]
    fn define_bits_lossless(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        reader: &mut SwfStream<&'a [u8]>,
        version: u8,
    ) -> DecodeResult {
        let define_bits_lossless = reader.read_define_bits_lossless(version)?;
        let bitmap_info = context.renderer.register_bitmap_png(&define_bits_lossless);
        let bitmap = crate::bitmap::Bitmap::new(
            context,
            define_bits_lossless.id,
            bitmap_info.handle,
            bitmap_info.width,
            bitmap_info.height,
        );
        context
            .library
            .register_character(define_bits_lossless.id, Character::Bitmap(Box::new(bitmap)));
        Ok(())
    }

    #[inline]
    fn define_morph_shape(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        reader: &mut SwfStream<&'a [u8]>,
        morph_shapes: &mut fnv::FnvHashMap<CharacterId, MorphShapeStatic>,
        version: u8,
    ) -> DecodeResult {
        // Certain backends may have to preload morph shape frames, so defer registering until the end.
        let swf_shape = reader.read_define_morph_shape(version)?;
        let morph_shape = MorphShapeStatic::from_swf_tag(context.renderer, &swf_shape);
        morph_shapes.insert(swf_shape.id, morph_shape);
        Ok(())
    }

    #[inline]
    fn define_shape(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        reader: &mut SwfStream<&'a [u8]>,
        version: u8,
    ) -> DecodeResult {
        let swf_shape = reader.read_define_shape(version)?;
        let graphic = Graphic::from_swf_tag(context, &swf_shape);
        context
            .library
            .register_character(swf_shape.id, Character::Graphic(Box::new(graphic)));
        Ok(())
    }

    #[inline]
    fn preload_place_object(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        reader: &mut SwfStream<&'a [u8]>,
        tag_len: usize,
        ids: &mut fnv::FnvHashMap<Depth, CharacterId>,
        morph_shapes: &mut fnv::FnvHashMap<CharacterId, MorphShapeStatic>,
        version: u8,
    ) -> DecodeResult {
        use swf::PlaceObjectAction;
        let place_object = if version == 1 {
            reader.read_place_object(tag_len)
        } else {
            reader.read_place_object_2_or_3(version)
        }?;
        match place_object.action {
            PlaceObjectAction::Place(id) => {
                if let Some(morph_shape) = morph_shapes.get_mut(&id) {
                    ids.insert(place_object.depth, id);
                    if let Some(ratio) = place_object.ratio {
                        morph_shape.register_ratio(context.renderer, ratio);
                    }
                }
            }
            PlaceObjectAction::Modify => {
                if let Some(&id) = ids.get(&place_object.depth) {
                    if let Some(morph_shape) = morph_shapes.get_mut(&id) {
                        ids.insert(place_object.depth, id);
                        if let Some(ratio) = place_object.ratio {
                            morph_shape.register_ratio(context.renderer, ratio);
                        }
                    }
                }
            }
            PlaceObjectAction::Replace(id) => {
                if let Some(morph_shape) = morph_shapes.get_mut(&id) {
                    ids.insert(place_object.depth, id);
                    if let Some(ratio) = place_object.ratio {
                        morph_shape.register_ratio(context.renderer, ratio);
                    }
                } else {
                    ids.remove(&place_object.depth);
                }
            }
        };

        Ok(())
    }

    #[inline]
    fn preload_sound_stream_block(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        reader: &mut SwfStream<&'a [u8]>,
        static_data: &mut MovieClipStatic,
        tag_len: usize,
    ) -> DecodeResult {
        if static_data.audio_stream_info.is_some() {
            let pos = reader.get_ref().position() as usize;
            let data = reader.get_ref().get_ref();
            let data = &data[pos..pos + tag_len];
            context.audio.preload_sound_stream_block(self.id(), data);
        }

        Ok(())
    }

    #[inline]
    fn preload_sound_stream_head(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        reader: &mut SwfStream<&'a [u8]>,
        static_data: &mut MovieClipStatic,
        _version: u8,
    ) -> DecodeResult {
        let audio_stream_info = reader.read_sound_stream_head()?;
        context
            .audio
            .preload_sound_stream_head(self.id(), &audio_stream_info);
        static_data.audio_stream_info = Some(audio_stream_info);
        Ok(())
    }

    #[inline]
    fn define_bits(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        reader: &mut SwfStream<&'a [u8]>,
        tag_len: usize,
    ) -> DecodeResult {
        use std::io::Read;
        let id = reader.read_u16()?;
        let data_len = tag_len - 2;
        let mut jpeg_data = Vec::with_capacity(data_len);
        reader
            .get_mut()
            .take(data_len as u64)
            .read_to_end(&mut jpeg_data)?;
        let bitmap_info = context.renderer.register_bitmap_jpeg(
            id,
            &jpeg_data,
            context.library.jpeg_tables().unwrap(),
        );
        let bitmap = crate::bitmap::Bitmap::new(
            context,
            id,
            bitmap_info.handle,
            bitmap_info.width,
            bitmap_info.height,
        );
        context
            .library
            .register_character(id, Character::Bitmap(Box::new(bitmap)));
        Ok(())
    }

    #[inline]
    fn define_bits_jpeg_2(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        reader: &mut SwfStream<&'a [u8]>,
        tag_len: usize,
    ) -> DecodeResult {
        use std::io::Read;
        let id = reader.read_u16()?;
        let data_len = tag_len - 2;
        let mut jpeg_data = Vec::with_capacity(data_len);
        reader
            .get_mut()
            .take(data_len as u64)
            .read_to_end(&mut jpeg_data)?;
        let bitmap_info = context.renderer.register_bitmap_jpeg_2(id, &jpeg_data);
        let bitmap = crate::bitmap::Bitmap::new(
            context,
            id,
            bitmap_info.handle,
            bitmap_info.width,
            bitmap_info.height,
        );
        context
            .library
            .register_character(id, Character::Bitmap(Box::new(bitmap)));
        Ok(())
    }

    #[inline]
    fn define_bits_jpeg_3(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        reader: &mut SwfStream<&'a [u8]>,
        tag_len: usize,
    ) -> DecodeResult {
        use std::io::Read;
        let id = reader.read_u16()?;
        let jpeg_len = reader.read_u32()? as usize;
        let alpha_len = tag_len - 6 - jpeg_len;
        let mut jpeg_data = Vec::with_capacity(jpeg_len);
        let mut alpha_data = Vec::with_capacity(alpha_len);
        reader
            .get_mut()
            .take(jpeg_len as u64)
            .read_to_end(&mut jpeg_data)?;
        reader
            .get_mut()
            .take(alpha_len as u64)
            .read_to_end(&mut alpha_data)?;
        let bitmap_info = context
            .renderer
            .register_bitmap_jpeg_3(id, &jpeg_data, &alpha_data);
        let bitmap = crate::bitmap::Bitmap::new(
            context,
            id,
            bitmap_info.handle,
            bitmap_info.width,
            bitmap_info.height,
        );
        context
            .library
            .register_character(id, Character::Bitmap(Box::new(bitmap)));
        Ok(())
    }

    #[inline]
    fn define_bits_jpeg_4(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        reader: &mut SwfStream<&'a [u8]>,
        tag_len: usize,
    ) -> DecodeResult {
        use std::io::Read;
        let id = reader.read_u16()?;
        let jpeg_len = reader.read_u32()? as usize;
        let _deblocking = reader.read_u16()?;
        let alpha_len = tag_len - 6 - jpeg_len;
        let mut jpeg_data = Vec::with_capacity(jpeg_len);
        let mut alpha_data = Vec::with_capacity(alpha_len);
        reader
            .get_mut()
            .take(jpeg_len as u64)
            .read_to_end(&mut jpeg_data)?;
        reader
            .get_mut()
            .take(alpha_len as u64)
            .read_to_end(&mut alpha_data)?;
        let bitmap_info = context
            .renderer
            .register_bitmap_jpeg_3(id, &jpeg_data, &alpha_data);
        let bitmap = crate::bitmap::Bitmap::new(
            context,
            id,
            bitmap_info.handle,
            bitmap_info.width,
            bitmap_info.height,
        );
        context
            .library
            .register_character(id, Character::Bitmap(Box::new(bitmap)));
        Ok(())
    }

    #[inline]
    fn define_button_1(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        reader: &mut SwfStream<&'a [u8]>,
    ) -> DecodeResult {
        let swf_button = reader.read_define_button_1()?;
        let button =
            crate::button::Button::from_swf_tag(&swf_button, &context.library, context.gc_context);
        context
            .library
            .register_character(swf_button.id, Character::Button(Box::new(button)));
        Ok(())
    }

    #[inline]
    fn define_button_2(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        reader: &mut SwfStream<&'a [u8]>,
    ) -> DecodeResult {
        let swf_button = reader.read_define_button_2()?;
        let button =
            crate::button::Button::from_swf_tag(&swf_button, &context.library, context.gc_context);
        context
            .library
            .register_character(swf_button.id, Character::Button(Box::new(button)));
        Ok(())
    }

    /// Defines a dynamic text field character.
    #[inline]
    fn define_edit_text(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        reader: &mut SwfStream<&'a [u8]>,
    ) -> DecodeResult {
        let swf_edit_text = reader.read_define_edit_text()?;
        let edit_text = crate::edit_text::EditText::from_swf_tag(context, swf_edit_text);
        context
            .library
            .register_character(edit_text.id(), Character::EditText(Box::new(edit_text)));
        Ok(())
    }

    #[inline]
    fn define_font_1(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        reader: &mut SwfStream<&'a [u8]>,
    ) -> DecodeResult {
        let font = reader.read_define_font_1()?;
        let glyphs = font
            .glyphs
            .into_iter()
            .map(|g| swf::Glyph {
                shape_records: g,
                code: 0,
                advance: None,
                bounds: None,
            })
            .collect::<Vec<_>>();

        let font = swf::Font {
            id: font.id,
            version: 0,
            name: "".to_string(),
            glyphs,
            language: swf::Language::Unknown,
            layout: None,
            is_small_text: false,
            is_shift_jis: false,
            is_ansi: false,
            is_bold: false,
            is_italic: false,
        };
        let font_object = Font::from_swf_tag(context.renderer, &font).unwrap();
        context
            .library
            .register_character(font.id, Character::Font(Box::new(font_object)));
        Ok(())
    }

    #[inline]
    fn define_font_2(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        reader: &mut SwfStream<&'a [u8]>,
    ) -> DecodeResult {
        let font = reader.read_define_font_2(2)?;
        let font_object = Font::from_swf_tag(context.renderer, &font).unwrap();
        context
            .library
            .register_character(font.id, Character::Font(Box::new(font_object)));
        Ok(())
    }

    #[inline]
    fn define_font_3(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        reader: &mut SwfStream<&'a [u8]>,
    ) -> DecodeResult {
        let font = reader.read_define_font_2(3)?;
        let font_object = Font::from_swf_tag(context.renderer, &font).unwrap();
        context
            .library
            .register_character(font.id, Character::Font(Box::new(font_object)));

        Ok(())
    }

    #[inline]
    fn define_sound(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        reader: &mut SwfStream<&'a [u8]>,
        tag_len: usize,
    ) -> DecodeResult {
        // TODO(Herschel): Can we use a slice of the sound data instead of copying the data?
        use std::io::Read;
        let mut reader =
            swf::read::Reader::new(reader.get_mut().take(tag_len as u64), context.swf_version);
        let sound = reader.read_define_sound()?;
        let handle = context.audio.register_sound(&sound).unwrap();
        context
            .library
            .register_character(sound.id, Character::Sound(handle));
        Ok(())
    }

    fn define_sprite(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        reader: &mut SwfStream<&'a [u8]>,
        tag_len: usize,
        morph_shapes: &mut fnv::FnvHashMap<CharacterId, MorphShapeStatic>,
    ) -> DecodeResult {
        let id = reader.read_character_id()?;
        let num_frames = reader.read_u16()?;
        let mut movie_clip = MovieClip::new_with_data(
            context.swf_version,
            context.gc_context,
            id,
            reader.get_ref().position(),
            tag_len - 4,
            num_frames,
        );

        movie_clip.preload(context, morph_shapes);

        context
            .library
            .register_character(id, Character::MovieClip(Box::new(movie_clip)));

        Ok(())
    }

    #[inline]
    fn define_text(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        reader: &mut SwfStream<&'a [u8]>,
        version: u8,
    ) -> DecodeResult {
        let text = reader.read_define_text(version)?;
        let text_object = Text::from_swf_tag(context, &text);
        context
            .library
            .register_character(text.id, Character::Text(Box::new(text_object)));
        Ok(())
    }

    #[inline]
    fn frame_label(
        &mut self,
        _context: &mut UpdateContext<'_, 'gc, '_>,
        reader: &mut SwfStream<&'a [u8]>,
        tag_len: usize,
        cur_frame: FrameNumber,
        static_data: &mut MovieClipStatic,
    ) -> DecodeResult {
        let frame_label = reader.read_frame_label(tag_len)?;
        if static_data
            .frame_labels
            .insert(frame_label.label, cur_frame)
            .is_some()
        {
            log::warn!("Movie clip {}: Duplicated frame label", self.id());
        }
        Ok(())
    }

    #[inline]
    fn jpeg_tables(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        reader: &mut SwfStream<&'a [u8]>,
        tag_len: usize,
    ) -> DecodeResult {
        use std::io::Read;
        // TODO(Herschel): Can we use a slice instead of copying?
        let mut jpeg_data = Vec::with_capacity(tag_len);
        reader
            .get_mut()
            .take(tag_len as u64)
            .read_to_end(&mut jpeg_data)?;
        context.library.set_jpeg_tables(jpeg_data);
        Ok(())
    }

    #[inline]
    fn preload_remove_object(
        &mut self,
        _context: &mut UpdateContext<'_, 'gc, '_>,
        reader: &mut SwfStream<&'a [u8]>,
        ids: &mut fnv::FnvHashMap<Depth, CharacterId>,
        version: u8,
    ) -> DecodeResult {
        let remove_object = if version == 1 {
            reader.read_remove_object_1()
        } else {
            reader.read_remove_object_2()
        }?;
        ids.remove(&remove_object.depth);
        Ok(())
    }

    #[inline]
    fn preload_show_frame(
        &mut self,
        _context: &mut UpdateContext<'_, 'gc, '_>,
        _reader: &mut SwfStream<&'a [u8]>,
        cur_frame: &mut FrameNumber,
    ) -> DecodeResult {
        *cur_frame += 1;
        Ok(())
    }
}

// Control tags
impl<'gc, 'a> MovieClip<'gc> {
    #[inline]
    fn do_action(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        reader: &mut SwfStream<&'a [u8]>,
        tag_len: usize,
    ) -> DecodeResult {
        // Queue the actions.
        // TODO: The reader is actually reading the tag slice at this point (tag_stream.take()),
        // so make sure to get the proper offsets. This feels kind of bad.
        let start = (self.tag_stream_start() + reader.get_ref().position()) as usize;
        let end = start + tag_len;
        let slice = crate::tag_utils::SwfSlice {
            data: std::sync::Arc::clone(context.swf_data),
            start,
            end,
        };
        context
            .action_queue
            .queue_actions(context.active_clip, slice);
        Ok(())
    }

    fn place_object(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        reader: &mut SwfStream<&'a [u8]>,
        tag_len: usize,
        version: u8,
    ) -> DecodeResult {
        let place_object = if version == 1 {
            reader.read_place_object(tag_len)
        } else {
            reader.read_place_object_2_or_3(version)
        }?;
        use swf::PlaceObjectAction;
        match place_object.action {
            PlaceObjectAction::Place(id) | PlaceObjectAction::Replace(id) => {
                if let Some(child) = self.instantiate_child(
                    context,
                    id,
                    place_object.depth,
                    if let PlaceObjectAction::Replace(_) = place_object.action {
                        true
                    } else {
                        false
                    },
                ) {
                    child
                        .write(context.gc_context)
                        .apply_place_object(&place_object);
                    child
                } else {
                    return Ok(());
                }
            }
            PlaceObjectAction::Modify => {
                if let Some(child) = self.children.get_mut(&place_object.depth) {
                    child
                        .write(context.gc_context)
                        .apply_place_object(&place_object);
                    *child
                } else {
                    return Ok(());
                }
            }
        };

        Ok(())
    }

    #[inline]
    fn remove_object(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        reader: &mut SwfStream<&'a [u8]>,
        version: u8,
    ) -> DecodeResult {
        let remove_object = if version == 1 {
            reader.read_remove_object_1()
        } else {
            reader.read_remove_object_2()
        }?;
        if let Some(child) = self.children.remove(&remove_object.depth) {
            self.remove_child_from_exec_list(context.gc_context, child);
        }
        Ok(())
    }

    #[inline]
    fn set_background_color(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        reader: &mut SwfStream<&'a [u8]>,
    ) -> DecodeResult {
        *context.background_color = reader.read_rgb()?;
        Ok(())
    }

    #[inline]
    fn sound_stream_block(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        _reader: &mut SwfStream<&'a [u8]>,
    ) -> DecodeResult {
        if let (Some(stream_info), None) = (&self.static_data.audio_stream_info, &self.audio_stream)
        {
            let slice = crate::tag_utils::SwfSlice {
                data: std::sync::Arc::clone(context.swf_data),
                start: self.tag_stream_start() as usize,
                end: self.tag_stream_start() as usize + self.tag_stream_len(),
            };
            let audio_stream = context.audio.start_stream(self.id(), slice, stream_info);
            self.audio_stream = Some(audio_stream);
        }

        Ok(())
    }

    #[inline]
    fn start_sound_1(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        reader: &mut SwfStream<&'a [u8]>,
    ) -> DecodeResult {
        let start_sound = reader.read_start_sound_1()?;
        if let Some(handle) = context.library.get_sound(start_sound.id) {
            use swf::SoundEvent;
            // The sound event type is controlled by the "Sync" setting in the Flash IDE.
            match start_sound.sound_info.event {
                // "Event" sounds always play, independent of the timeline.
                SoundEvent::Event => context.audio.start_sound(handle, &start_sound.sound_info),

                // "Start" sounds only play if an instance of the same sound is not already playing.
                SoundEvent::Start => {
                    if !context.audio.is_sound_playing_with_handle(handle) {
                        context.audio.start_sound(handle, &start_sound.sound_info);
                    }
                }

                // "Stop" stops any active instances of a given sound.
                SoundEvent::Stop => context.audio.stop_sounds_with_handle(handle),
            }
        }
        Ok(())
    }
}

/// Static data shared between all instances of a movie clip.
#[allow(dead_code)]
#[derive(Clone)]
struct MovieClipStatic {
    id: CharacterId,
    tag_stream_start: u64,
    tag_stream_len: usize,
    frame_labels: HashMap<String, FrameNumber>,
    audio_stream_info: Option<swf::SoundStreamHead>,
    total_frames: FrameNumber,
}

impl Default for MovieClipStatic {
    fn default() -> Self {
        Self {
            id: 0,
            tag_stream_start: 0,
            tag_stream_len: 0,
            total_frames: 1,
            frame_labels: HashMap::new(),
            audio_stream_info: None,
        }
    }
}

unsafe impl<'gc> gc_arena::Collect for MovieClipStatic {
    #[inline]
    fn needs_trace() -> bool {
        false
    }
}

/// Stores the placement settings for display objects during a
/// goto command.
#[derive(Debug)]
struct GotoPlaceObject {
    /// The frame number that this character was first placed on.
    frame: FrameNumber,
    /// The display properties of the object.
    place_object: swf::PlaceObject,
}

impl GotoPlaceObject {
    #[inline]
    fn id(&self) -> CharacterId {
        match &self.place_object.action {
            swf::PlaceObjectAction::Place(id) | swf::PlaceObjectAction::Replace(id) => *id,
            swf::PlaceObjectAction::Modify => 0,
        }
    }

    #[inline]
    fn modifies_original_item(&self) -> bool {
        if let swf::PlaceObjectAction::Replace(_) = &self.place_object.action {
            true
        } else {
            false
        }
    }

    fn merge(&mut self, next: &mut GotoPlaceObject) {
        use swf::PlaceObjectAction;
        let cur_place = &mut self.place_object;
        let next_place = &mut next.place_object;
        match (cur_place.action, next_place.action) {
            (cur, PlaceObjectAction::Modify) => {
                cur_place.action = cur;
            }
            (_, new) => {
                cur_place.action = new;
                self.frame = next.frame;
            }
        };
        if next_place.matrix.is_some() {
            cur_place.matrix = next_place.matrix.take();
        }
        if next_place.color_transform.is_some() {
            cur_place.color_transform = next_place.color_transform.take();
        }
        if next_place.ratio.is_some() {
            cur_place.ratio = next_place.ratio.take();
        }
        if next_place.name.is_some() {
            cur_place.name = next_place.name.take();
        }
        if next_place.clip_depth.is_some() {
            cur_place.clip_depth = next_place.clip_depth.take();
        }
        if next_place.class_name.is_some() {
            cur_place.class_name = next_place.class_name.take();
        }
        if next_place.background_color.is_some() {
            cur_place.background_color = next_place.background_color.take();
        }
        // TODO: Other stuff.
    }
}
