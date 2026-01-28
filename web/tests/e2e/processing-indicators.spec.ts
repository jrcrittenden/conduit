import { test, expect } from '@playwright/test';
import { mockApi, sessionId } from './fixtures';
import { installMockWebSocket } from './websocket-mock';

test.beforeEach(async ({ page }) => {
  await mockApi(page);
  await installMockWebSocket(page);
});

test('processing indicators stay hidden on TurnStarted', async ({ page }) => {
  await page.goto('/');
  await page.waitForResponse('**/api/bootstrap');
  await expect(page.getByPlaceholder('Type a message...')).toBeVisible();

  await page.evaluate((id) => {
    const ws = (window as unknown as { __mockWebSocket?: { sendMessage: (msg: unknown) => void } })
      .__mockWebSocket;
    ws?.sendMessage({ type: 'agent_event', session_id: id, event: { type: 'TurnStarted' } });
  }, sessionId);

  await expect(page.getByText('Processing...')).toHaveCount(0);
  await expect(page.locator('[aria-label=\"Processing\"]')).toHaveCount(0);
});

test('esc hint and stop button reflect processing state', async ({ page }) => {
  await page.goto('/');
  await page.waitForResponse('**/api/bootstrap');
  await expect(page.getByPlaceholder('Type a message...')).toBeVisible();

  const stopButton = page.getByRole('button', { name: 'Stop session' });
  await expect(stopButton).toHaveCount(0);

  await page.evaluate((id) => {
    const ws = (window as unknown as { __mockWebSocket?: { sendMessage: (msg: unknown) => void } })
      .__mockWebSocket;
    ws?.sendMessage({ type: 'agent_event', session_id: id, event: { type: 'TurnStarted' } });
  }, sessionId);

  await expect(stopButton).toHaveCount(1);

  const input = page.locator('textarea[data-chat-input="true"]');
  await input.click();
  await page.keyboard.press('Escape');
  await expect(page.getByText('Press Esc again to interrupt')).toBeVisible();

  await page.keyboard.press('Escape');
  await expect(page.getByText('Press Esc again to interrupt')).toHaveCount(0);
  await expect(stopButton).toHaveCount(0);
});

test('stop clears processing state immediately', async ({ page }) => {
  await page.goto('/');
  await page.waitForResponse('**/api/bootstrap');
  await expect(page.getByPlaceholder('Type a message...')).toBeVisible();

  const stopButton = page.getByRole('button', { name: 'Stop session' });
  await page.evaluate((id) => {
    const ws = (window as unknown as { __mockWebSocket?: { sendMessage: (msg: unknown) => void } })
      .__mockWebSocket;
    ws?.sendMessage({ type: 'agent_event', session_id: id, event: { type: 'TurnStarted' } });
  }, sessionId);

  await expect(stopButton).toHaveCount(1);

  const input = page.locator('textarea[data-chat-input="true"]');
  await input.click();
  await page.keyboard.press('Escape');
  await page.keyboard.press('Escape');

  await expect(stopButton).toHaveCount(0);
  await expect(page.getByPlaceholder('Type a message...')).toBeVisible();
  await expect(page.getByPlaceholder('Waiting for response...')).toHaveCount(0);
});

test('stop button clears processing state without TurnCompleted', async ({ page }) => {
  await page.goto('/');
  await page.waitForResponse('**/api/bootstrap');
  await expect(page.getByPlaceholder('Type a message...')).toBeVisible();

  const stopButton = page.getByRole('button', { name: 'Stop session' });
  await expect(stopButton).toHaveCount(0);

  await page.evaluate((id) => {
    const ws = (window as unknown as { __mockWebSocket?: { sendMessage: (msg: unknown) => void } })
      .__mockWebSocket;
    ws?.sendMessage({ type: 'agent_event', session_id: id, event: { type: 'TurnStarted' } });
  }, sessionId);

  await expect(stopButton).toHaveCount(1);
  await stopButton.click();
  await expect(stopButton).toHaveCount(0);
});

