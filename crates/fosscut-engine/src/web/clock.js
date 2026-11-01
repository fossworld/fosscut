// FossCut's clock and time control for web compositions (ADR-0039). The engine installs it
// before any script of the page runs, then calls __fosscut.ready() once the page has loaded
// and __fosscut.show(frame, num, den) before each screenshot.
//
// The page's clock is composition time: performance.now(), Date, the timestamps given to
// requestAnimationFrame callbacks and timers all read it. It starts at 0 and stands still
// until FossCut moves it. Without a seek hook, FossCut steps it one composition frame at a
// time: due timers run, then one round of animation frame callbacks, so frame n is always
// the result of the same steps. CSS animations, CSS transitions and Web Animations are
// paused and placed at the time since they appeared; SVG animations and media elements
// follow the clock. A page that defines window.fosscut.seek(seconds) is moved straight to
// each frame's time instead and calls the hook, whose promise FossCut awaits.
(() => {
  'use strict';
  if (window.__fosscut) {
    return;
  }
  const realSetTimeout = window.setTimeout.bind(window);
  const RealDate = window.Date;
  // Date.now() at composition time 0: 2000-01-01T00:00:00Z.
  const EPOCH = 946684800000;
  // Longest wait for fonts, images, media seeks and a seek hook, in milliseconds.
  const PATIENCE = 15000;

  let now = 0;
  let frame = -1;
  let stateful = false;
  let seq = 0;
  let num = 30;
  let den = 1;
  let gsapSeen = false;
  const frames = new Map();
  const timers = new Map();
  const animations = [];
  const births = new WeakMap();
  const errors = [];

  const report = (error) => {
    if (errors.length < 20) {
      errors.push(String((error && (error.stack || error.message)) || error));
    }
  };
  const call = (fn, args) => {
    try {
      if (typeof fn === 'function') {
        fn(...args);
      } else {
        (0, eval)(String(fn));
      }
    } catch (error) {
      report(error);
    }
  };

  // The clock.
  function FossCutDate(...args) {
    if (!new.target) {
      return new RealDate(EPOCH + now).toString();
    }
    return args.length ? new RealDate(...args) : new RealDate(EPOCH + now);
  }
  FossCutDate.prototype = RealDate.prototype;
  FossCutDate.now = () => EPOCH + now;
  FossCutDate.parse = RealDate.parse;
  FossCutDate.UTC = RealDate.UTC;
  window.Date = FossCutDate;
  performance.now = () => now;

  window.requestAnimationFrame = (callback) => {
    stateful = true;
    seq += 1;
    frames.set(seq, callback);
    return seq;
  };
  window.cancelAnimationFrame = (id) => {
    frames.delete(id);
  };

  // Timers run when the clock reaches them; those due now run on the next turn.
  let pumping = false;
  const pump = () => {
    if (!pumping) {
      pumping = true;
      realSetTimeout(() => {
        pumping = false;
        runTimers(now);
      }, 0);
    }
  };
  const schedule = (fn, delay, args, repeat) => {
    stateful = true;
    seq += 1;
    const wait = Math.max(0, Number(delay) || 0);
    timers.set(seq, { id: seq, due: now + wait, order: seq, fn, args, every: repeat ? Math.max(1, wait) : 0 });
    if (wait === 0) {
      pump();
    }
    return seq;
  };
  window.setTimeout = (fn, delay, ...args) => schedule(fn, delay, args, false);
  window.setInterval = (fn, delay, ...args) => schedule(fn, delay, args, true);
  window.clearTimeout = (id) => {
    timers.delete(id);
  };
  window.clearInterval = window.clearTimeout;

  const runTimers = (until) => {
    for (let guard = 0; guard < 100000; guard += 1) {
      let next = null;
      for (const timer of timers.values()) {
        if (timer.due <= until && (!next || timer.due < next.due || (timer.due === next.due && timer.order < next.order))) {
          next = timer;
        }
      }
      if (!next) {
        return;
      }
      now = Math.max(now, next.due);
      if (next.every) {
        next.due += next.every;
        seq += 1;
        next.order = seq;
      } else {
        timers.delete(next.id);
      }
      call(next.fn, next.args);
    }
  };

  // One macrotask turn: promise reactions and message events queued so far run.
  const channel = new MessageChannel();
  const turns = [];
  channel.port1.onmessage = () => {
    const resolve = turns.shift();
    if (resolve) {
      resolve();
    }
  };
  const settle = () => new Promise((resolve) => {
    turns.push(resolve);
    channel.port2.postMessage(null);
  });
  const patience = (promise) => Promise.race([
    promise,
    new Promise((resolve) => realSetTimeout(resolve, PATIENCE)),
  ]);

  const at = (k) => (k * 1000 * den) / num;

  const runFrames = (time) => {
    const callbacks = [...frames.values()];
    frames.clear();
    for (const callback of callbacks) {
      call(callback, [time]);
    }
  };

  // Animations: every animation is paused and placed at the time since it appeared.
  const notice = () => {
    for (const animation of document.getAnimations()) {
      if (!births.has(animation)) {
        births.set(animation, now);
        animations.push(animation);
        if (now > 0) {
          stateful = true;
        }
      }
    }
  };
  const place = () => {
    notice();
    for (const animation of animations) {
      if (animation.playState === 'idle') {
        continue;
      }
      try {
        animation.pause();
        animation.currentTime = Math.max(0, now - births.get(animation));
      } catch (error) {
        report(error);
      }
    }
    for (const svg of document.querySelectorAll('svg')) {
      if (!svg.ownerSVGElement && typeof svg.setCurrentTime === 'function') {
        svg.pauseAnimations();
        svg.setCurrentTime(now / 1000);
      }
    }
  };
  const gsapOff = () => {
    // A jump of the clock must never be shortened by GSAP's lag smoothing.
    const gsap = window.gsap;
    if (!gsapSeen && gsap && gsap.ticker && typeof gsap.ticker.lagSmoothing === 'function') {
      gsap.ticker.lagSmoothing(0);
      gsapSeen = true;
    }
  };

  // Fonts, images and media at the shown time.
  const images = () => Promise.all([...document.images].map((image) => {
    if (image.complete) {
      return image.decode ? image.decode().catch(() => {}) : null;
    }
    return new Promise((resolve) => {
      image.addEventListener('load', resolve, { once: true });
      image.addEventListener('error', resolve, { once: true });
    });
  }));
  const media = () => Promise.all([...document.querySelectorAll('video, audio')].map((element) => {
    element.muted = true;
    element.pause();
    if (!(element.duration > 0)) {
      return null;
    }
    let target = now / 1000;
    target = element.loop ? target % element.duration : Math.min(target, element.duration);
    if (Math.abs(element.currentTime - target) < 0.0005) {
      return null;
    }
    return new Promise((resolve) => {
      element.addEventListener('seeked', resolve, { once: true });
      element.addEventListener('error', resolve, { once: true });
      element.currentTime = target;
    });
  }));
  const loaded = async () => {
    // Reading layout makes the page ask for the fonts it uses.
    void document.documentElement.getBoundingClientRect();
    await patience(Promise.all([document.fonts.ready, images()]));
  };

  const hook = () => {
    const page = window.fosscut;
    return page && typeof page.seek === 'function' ? page.seek.bind(page) : null;
  };

  const step = async (k) => {
    const time = at(k);
    runTimers(time);
    now = time;
    gsapOff();
    runFrames(time);
    await settle();
    place();
    frame = k;
  };

  Object.defineProperty(window, '__fosscut', {
    enumerable: false,
    value: Object.freeze({
      // Called once the page has loaded: waits for its fonts and images.
      ready: async () => {
        await loaded();
        gsapOff();
        place();
        return 'ok';
      },
      // Shows frame k of a composition at num/den frames per second. Answers 'reload'
      // when the page must be loaded again to go back in time.
      show: async (k, rateNum, rateDen) => {
        num = rateNum;
        den = rateDen;
        const seek = hook();
        if (seek) {
          now = at(k);
          frame = k;
          place();
          const result = seek(now / 1000);
          if (result && typeof result.then === 'function') {
            let done = false;
            result.then(() => { done = true; }, () => { done = true; });
            // Animation frames the hook waits for run at the frame's time meanwhile.
            const started = RealDate.now();
            while (!done && RealDate.now() - started < PATIENCE) {
              runFrames(now);
              await settle();
            }
            await patience(result);
          }
          runFrames(now);
          await settle();
        } else if (k < frame) {
          if (stateful) {
            return 'reload';
          }
          now = at(k);
          frame = k;
          place();
        } else {
          while (frame < k) {
            await step(frame + 1);
          }
          place();
        }
        await patience(media());
        await loaded();
        return 'ok';
      },
      // Errors thrown by the page's callbacks so far.
      errors: () => errors.slice(),
    }),
  });
})();
