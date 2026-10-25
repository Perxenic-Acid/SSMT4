export type MutationImpact = 'content' | 'structure';

interface MutationReconcilerOptions {
    getGameName: () => string;
    invalidateClientState: (gameName: string, impact: MutationImpact) => void;
    refreshCurrentContent: (gameName: string) => Promise<void>;
    refreshStructure: (gameName: string) => Promise<void>;
    onError?: (error: unknown, impact: MutationImpact) => void;
}

const strongerImpact = (left: MutationImpact | null, right: MutationImpact): MutationImpact => {
    if (left === 'structure' || right === 'structure') return 'structure';
    return 'content';
};

export const createMutationReconciler = (options: MutationReconcilerOptions) => {
    let requestedImpact: MutationImpact | null = null;
    let scheduled = false;
    let running = false;
    let waiters: Array<() => void> = [];

    const schedule = () => {
        if (scheduled || running || !requestedImpact) return;
        scheduled = true;
        queueMicrotask(() => { void flush(); });
    };

    const flush = async () => {
        scheduled = false;
        if (running || !requestedImpact) return;

        running = true;
        const impact = requestedImpact;
        requestedImpact = null;
        const currentWaiters = waiters;
        waiters = [];

        try {
            const gameName = options.getGameName();
            if (gameName) {
                options.invalidateClientState(gameName, impact);
                if (impact === 'structure') {
                    await options.refreshStructure(gameName);
                } else {
                    await options.refreshCurrentContent(gameName);
                }
            }
        } catch (error) {
            options.onError?.(error, impact);
        } finally {
            running = false;
            currentWaiters.forEach((resolve) => resolve());
            schedule();
        }
    };

    const reconcile = (impact: MutationImpact = 'content') => {
        requestedImpact = strongerImpact(requestedImpact, impact);
        const completion = new Promise<void>((resolve) => waiters.push(resolve));
        schedule();
        return completion;
    };

    return { reconcile };
};
