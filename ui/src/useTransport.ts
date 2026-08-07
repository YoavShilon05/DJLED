import { useCallback, useEffect, useRef, useState } from "react";

/**
 * The shared clock.
 *
 * One clock for the whole show, because there is one strip and therefore one
 * "now". Layers take their phase from it rather than keeping clocks of their
 * own, which is what makes a 2 s loop and an 8 s loop stay in step instead of
 * drifting apart at a rate nobody chose.
 *
 * Driven by `requestAnimationFrame` against wall-clock time rather than by
 * accumulating a fixed step per frame: a dropped frame then costs a frame of
 * smoothness rather than putting the animation permanently behind.
 */
export function useTransport(duration: number) {
  const [time, setTime] = useState(0);
  const [playing, setPlaying] = useState(false);

  // Read inside the animation callback, so changing either does not tear the
  // loop down and start a new one mid-play.
  const durationRef = useRef(duration);
  durationRef.current = duration;

  useEffect(() => {
    if (!playing) return;
    let raf = 0;
    let last = performance.now();

    const tick = (now: number) => {
      raf = requestAnimationFrame(tick);
      const dt = (now - last) / 1000;
      last = now;
      setTime((t) => {
        const span = durationRef.current;
        if (!(span > 0)) return 0;
        return (t + dt) % span;
      });
    };

    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [playing]);

  // A scrub past the end of a shortened transport would otherwise strand the
  // playhead somewhere the scrubber cannot reach.
  useEffect(() => {
    setTime((t) => (duration > 0 && t >= duration ? 0 : t));
  }, [duration]);

  const seek = useCallback((next: number) => {
    setTime(Math.max(0, next));
  }, []);

  return { time, playing, setPlaying, seek };
}
