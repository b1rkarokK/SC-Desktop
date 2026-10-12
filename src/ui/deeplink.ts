// Links `scdesk://room/…` and `scdesk://playlist/…` that Windows hands to the program: ask, then join.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { errorMessage } from './api';
import { confirmDialog } from './confirm';
import { toast } from './dom';
import { T } from './i18n';
import { router } from './router';

async function open(url: string): Promise<void> {
  const room = url.toLowerCase().startsWith('scdesk://room/');
  const ok = await confirmDialog(room ? T('Присоединиться к комнате по ссылке?') : T('Присоединиться к общему плейлисту по ссылке?'), room ? T('Вы зайдёте в комнату, где слушают вместе.') : T('Плейлист появится в разделе «Общие плейлисты».'), T('Присоединиться'));
  if (!ok) return;
  try {
    if (room) {
      await invoke('room_join', { link: url, password: '' });
      router.go({ name: 'rooms' });
    } else {
      await invoke('sp_join', { link: url });
      router.go({ name: 'shared' });
    }
  } catch (e) {
    toast(errorMessage(e), 'error');
  }
}

export function initDeepLinks(): void {
  void listen<string>('deeplink', (e) => void open(e.payload));
  // a link that started the program
  void invoke<string | null>('deeplink_take').then((url) => url && void open(url));
}
