import { ref, onUnmounted } from "vue";

/** A boolean that flips true then auto-resets after `duration` ms — for
 *  transient "Copied!" / "Saved!" confirmations. Re-triggering restarts the
 *  timer; the timer is cleared on unmount to avoid touching a dead component. */
export function useTemporaryFlag(duration = 1500) {
  const flag = ref(false);
  let timer: ReturnType<typeof setTimeout> | null = null;

  function trigger() {
    flag.value = true;
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
      flag.value = false;
      timer = null;
    }, duration);
  }

  onUnmounted(() => {
    if (timer) clearTimeout(timer);
  });

  return { flag, trigger };
}
