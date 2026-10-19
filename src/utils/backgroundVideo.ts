// 后台暂停与用户暂停分开记录，避免切回窗口时擅自恢复用户暂停的视频。
export function attachBackgroundVideo(
  video: HTMLVideoElement,
  visibility: Pick<Document, 'hidden' | 'addEventListener' | 'removeEventListener'>,
  onFailure: () => void,
  // 浏览器计时器必须以 Window 为接收者，不能直接挂到 clock 后作为其方法调用。
  clock = {
    now: () => performance.now(),
    setInterval: (callback: () => void, delay: number) => globalThis.setInterval(callback, delay),
    clearInterval: (timer: ReturnType<typeof setInterval>) => globalThis.clearInterval(timer),
  },
) {
  let disposed = false;
  let started = false;
  let suspended = false;
  let lastProgress = clock.now();
  let lastTime = video.currentTime;
  let frame = 0;
  let lastFrame = 0;
  let frameCallback: number | undefined;
  const hasFrameCallback = typeof video.requestVideoFrameCallback === 'function';
  const trackFrame = () => {
    frame += 1;
    if (!disposed) frameCallback = video.requestVideoFrameCallback(trackFrame);
  };
  const listen = (event: string, handler: EventListener) => video.addEventListener(event, handler);
  const fail = () => {
    if (disposed) return;
    stop();
    onFailure();
  };
  const play = () => {
    lastProgress = clock.now();
    void video.play().catch(() => {
      if (!disposed && !visibility.hidden) fail();
    });
  };
  const playing = () => { started = true; lastProgress = clock.now(); };
  const onVisibility = () => {
    if (visibility.hidden) {
      suspended = !video.paused || !started;
      video.pause();
    } else if (suspended) {
      suspended = false;
      play();
    }
    lastProgress = clock.now();
  };
  const timer = clock.setInterval(() => {
    if (visibility.hidden || (started && video.paused)) {
      lastProgress = clock.now();
      return;
    }
    const progressed = hasFrameCallback ? frame !== lastFrame : video.currentTime !== lastTime;
    if (progressed) lastProgress = clock.now();
    lastTime = video.currentTime;
    lastFrame = frame;
    // 元数据与首帧最多等待 15 秒；持续无新画面 10 秒后释放解码器并显示海报。
    if (clock.now() - lastProgress >= (started ? 10_000 : 15_000)) fail();
  }, 1000);
  function stop() {
    if (disposed) return;
    disposed = true;
    clock.clearInterval(timer);
    visibility.removeEventListener('visibilitychange', onVisibility);
    video.removeEventListener('error', fail);
    video.removeEventListener('playing', playing);
    if (frameCallback !== undefined) video.cancelVideoFrameCallback(frameCallback);
    video.pause();
    video.removeAttribute('src');
    // 单纯移除 DOM 不保证 WebView 及时释放媒体解码资源。
    video.load();
  }
  listen('error', fail);
  listen('playing', playing);
  visibility.addEventListener('visibilitychange', onVisibility);
  video.muted = true;
  if (hasFrameCallback) frameCallback = video.requestVideoFrameCallback(trackFrame);
  if (visibility.hidden) suspended = true;
  else play();
  return stop;
}
