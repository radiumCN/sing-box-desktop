import { ref } from "vue";

/** Drives a manual "refresh" button: keeps the spinning state visible for at
 *  least `minSpinMs` so a fast refresh still reads as a deliberate action.
 *  Reentrancy-guarded — concurrent calls while refreshing are ignored. */
export function useDelayedRefresh(minSpinMs = 600) {
  const refreshing = ref(false);

  async function refresh(fetchFn: () => Promise<unknown> | unknown): Promise<void> {
    if (refreshing.value) return;
    refreshing.value = true;
    try {
      await Promise.all([
        Promise.resolve(fetchFn()),
        new Promise((r) => setTimeout(r, minSpinMs)),
      ]);
    } finally {
      refreshing.value = false;
    }
  }

  return { refreshing, refresh };
}
