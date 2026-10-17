import { get, post, setUnauthorizedHandler, type User } from './api';

type State = { status: 'loading' } | { status: 'setup' } | { status: 'signed-out' } | { status: 'signed-in'; user: User };

export const session = $state<{ current: State }>({ current: { status: 'loading' } });

setUnauthorizedHandler(() => {
  session.current = { status: 'signed-out' };
});

export async function loadSession(): Promise<void> {
  try {
    const { needs_setup } = await get<{ needs_setup: boolean }>('/setup');
    if (needs_setup) {
      session.current = { status: 'setup' };
      return;
    }
    const user = await get<User>('/auth/me');
    session.current = { status: 'signed-in', user };
  } catch {
    if (session.current.status === 'loading') session.current = { status: 'signed-out' };
  }
}

export function signedIn(user: User): void {
  session.current = { status: 'signed-in', user };
}

export async function signOut(): Promise<void> {
  try {
    await post('/auth/logout');
  } finally {
    session.current = { status: 'signed-out' };
  }
}
