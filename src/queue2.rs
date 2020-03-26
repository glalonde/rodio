use std::time::Duration;

use source::Empty;
use source::Source;
use source::Zero;

use Sample;

#[derive(Debug)]
enum MusicPlayerCommand {
    Play,
    Pause,
    Stop,
    NextTrack,
}

pub struct SourcesQueueController<S> {
    command_channel: std::sync::mpsc::Sender<MusicPlayerCommand>,
    sound_channel: std::sync::mpsc::Sender<Box<dyn Source<Item = S> + Send>>,
}

impl<S> SourcesQueueController<S>
where
    S: Sample + Send + 'static,
{
    /// Adds a new source to the end of the queue.
    #[inline]
    pub fn append<T>(&self, source: T)
    where
        T: Source<Item = S> + Send + 'static,
    {
        let _ = self.sound_channel.send(Box::new(source) as Box<_>);
    }

    pub fn pause(&self) {
        let _ = self.command_channel.send(MusicPlayerCommand::Pause);
    }

    pub fn play(&self) {
        let _ = self.command_channel.send(MusicPlayerCommand::Play);
    }

    pub fn next(&self) {
        let _ = self.command_channel.send(MusicPlayerCommand::NextTrack);
    }

    pub fn stop(&self) {
        let _ = self.command_channel.send(MusicPlayerCommand::Stop);
    }
}

pub fn queue2<S>(keep_alive_if_empty: bool) -> (SourcesQueueController<S>, SourcesQueue<S>)
where
    S: Sample + Send + 'static,
{
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<MusicPlayerCommand>();
    let (source_tx, source_rx) = std::sync::mpsc::channel::<Box<dyn Source<Item = S> + Send>>();
    let output = SourcesQueue {
        sound_queue: Vec::new(),
        current: Box::new(Empty::<S>::new()) as Box<_>,
        keep_alive_if_empty,
        command_channel: cmd_rx,
        sound_channel: source_rx,
        paused: false,
    };
    let input = SourcesQueueController {
        command_channel: cmd_tx,
        sound_channel: source_tx,
    };

    (input, output)
}

/// The input of the queue.
pub struct SourcesQueue<S> {
    sound_queue: Vec<Box<dyn Source<Item = S> + Send>>,

    current: Box<dyn Source<Item = S> + Send>,

    keep_alive_if_empty: bool,

    command_channel: std::sync::mpsc::Receiver<MusicPlayerCommand>,

    sound_channel: std::sync::mpsc::Receiver<Box<dyn Source<Item = S> + Send>>,

    paused: bool,
}

impl<S> Source for SourcesQueue<S>
where
    S: Sample + Send + 'static,
{
    #[inline]
    fn current_frame_len(&self) -> Option<usize> {
        // This function is non-trivial because the boundary between two sounds in the queue should
        // be a frame boundary as well.
        //
        // The current sound is free to return `None` for `current_frame_len()`, in which case
        // we *should* return the number of samples remaining the current sound.
        // This can be estimated with `size_hint()`.
        //
        // If the `size_hint` is `None` as well, we are in the worst case scenario. To handle this
        // situation we force a frame to have a maximum number of samples indicate by this
        // constant.
        const THRESHOLD: usize = 512;

        // Try the current `current_frame_len`.
        if let Some(val) = self.current.current_frame_len() {
            if val != 0 {
                return Some(val);
            }
        }

        // Try the size hint.
        if let Some(val) = self.current.size_hint().1 {
            if val < THRESHOLD && val != 0 {
                return Some(val);
            }
        }

        // Otherwise we use the constant value.
        Some(THRESHOLD)
    }

    #[inline]
    fn channels(&self) -> u16 {
        self.current.channels()
    }

    #[inline]
    fn sample_rate(&self) -> u32 {
        self.current.sample_rate()
    }

    #[inline]
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

impl<S> Iterator for SourcesQueue<S>
where
    S: Sample + Send + 'static,
{
    type Item = S;

    #[inline]
    fn next(&mut self) -> Option<S> {
        loop {
            // Read command channel.
            self.read_command_channel();

            // Read input channel.
            self.read_sound_channel();

            if self.paused {
                return Some(S::zero_value());
            }

            // Basic situation that will happen most of the time.
            if let Some(sample) = self.current.next() {
                return Some(sample);
            }

            // Since `self.current` has finished, we need to pick the next sound.
            // In order to avoid inlining this expensive operation, the code is in another function.
            if self.go_next().is_err() {
                return None;
            }
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.current.size_hint().0, None)
    }
}

impl<S> SourcesQueue<S>
where
    S: Sample + Send + 'static,
{
    fn read_command_channel(&mut self) {
        // Read one command per sample.
        match self.command_channel.try_recv() {
            Ok(command) => self.handle_command(command),
            Err(_) => (),
        }
    }

    fn handle_command(&mut self, command: MusicPlayerCommand) {
        println!("Got command! {:?}", command);

        match command {
            MusicPlayerCommand::Play => {
                self.paused = false;
            }
            MusicPlayerCommand::Pause => {
                self.paused = true;
            }
            MusicPlayerCommand::NextTrack => {
                let _ = self.go_next();
            }
            MusicPlayerCommand::Stop => {
                self.sound_queue.clear();
                let _ = self.go_next();
            }
        };
    }

    fn read_sound_channel(&mut self) {
        match self.sound_channel.try_recv() {
            Ok(source) => self.sound_queue.push(source),
            Err(_) => (),
        }
    }

    // Called when `current` is empty and we must jump to the next element.
    // Returns `Ok` if the sound should continue playing, or an error if it should stop.
    //
    // This method is separate so that it is not inlined.
    fn go_next(&mut self) -> Result<(), ()> {
        let next = {
            if self.sound_queue.len() == 0 {
                if self.keep_alive_if_empty {
                    // Play a short silence in order to avoid spinlocking.
                    let silence = Zero::<S>::new(1, 44100); // TODO: meh
                    Box::new(silence.take_duration(Duration::from_millis(10))) as Box<_>
                } else {
                    return Err(());
                }
            } else {
                self.sound_queue.remove(0)
            }
        };

        self.current = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use buffer::SamplesBuffer;
    use queue2;
    use source::Source;

    #[test]
    #[ignore] // FIXME: samples rate and channel not updated immediately after transition
    fn basic() {
        let (tx, mut rx) = queue2::queue2(false);

        tx.append(SamplesBuffer::new(1, 48000, vec![10i16, -10, 10, -10]));
        tx.append(SamplesBuffer::new(2, 96000, vec![5i16, 5, 5, 5]));

        assert_eq!(rx.channels(), 1);
        assert_eq!(rx.sample_rate(), 48000);
        assert_eq!(rx.next(), Some(10));
        assert_eq!(rx.next(), Some(-10));
        assert_eq!(rx.next(), Some(10));
        assert_eq!(rx.next(), Some(-10));
        assert_eq!(rx.channels(), 2);
        assert_eq!(rx.sample_rate(), 96000);
        assert_eq!(rx.next(), Some(5));
        assert_eq!(rx.next(), Some(5));
        assert_eq!(rx.next(), Some(5));
        assert_eq!(rx.next(), Some(5));
        assert_eq!(rx.next(), None);
    }

    #[test]
    fn immediate_end() {
        let (_, mut rx) = queue2::queue2::<i16>(false);
        assert_eq!(rx.next(), None);
    }

    #[test]
    fn keep_alive() {
        let (tx, mut rx) = queue2::queue2(true);
        tx.append(SamplesBuffer::new(1, 48000, vec![10i16, -10, 10, -10]));

        assert_eq!(rx.next(), Some(10));
        assert_eq!(rx.next(), Some(-10));
        assert_eq!(rx.next(), Some(10));
        assert_eq!(rx.next(), Some(-10));

        for _ in 0..100000 {
            assert_eq!(rx.next(), Some(0));
        }
    }

    #[test]
    #[ignore] // TODO: not yet implemented
    fn no_delay_when_added() {
        let (tx, mut rx) = queue2::queue2(true);

        for _ in 0..500 {
            assert_eq!(rx.next(), Some(0));
        }

        tx.append(SamplesBuffer::new(1, 48000, vec![10i16, -10, 10, -10]));
        assert_eq!(rx.next(), Some(10));
        assert_eq!(rx.next(), Some(-10));
        assert_eq!(rx.next(), Some(10));
        assert_eq!(rx.next(), Some(-10));
    }
}
