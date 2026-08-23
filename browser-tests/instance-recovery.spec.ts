import { expect, test } from '@playwright/test'

test.afterEach(async ({ page }) => {
  await expect(page.locator('html')).toHaveAttribute('data-tauri-mock-unhandled', '[]')
})

test('failure recovery policy, incident evidence, and cancellation stay operator-visible', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'en-US')
    localStorage.setItem('lastTab', 'instances')
  })
  await page.goto('/')
  await expect(page.locator('html')).toHaveAttribute(
    'data-tauri-browser-test',
    '__LLAMA_MANAGER_BROWSER_TEST_BACKEND__',
  )

  const recoveryPolicy = page.getByTitle('Failure Recovery')
  await expect(recoveryPolicy).toHaveAttribute('aria-checked', 'false')
  await recoveryPolicy.click()
  await expect(recoveryPolicy).toHaveAttribute('aria-checked', 'true')
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.state.instances['browser-test-instance'].restart_policy
  ))).toBe('on-failure')

  const occurredAt = Math.floor(Date.now() / 1000)
  await page.evaluate(async ({ occurredAt }) => {
    const failure = {
      kind: 'unexpected_exit',
      message: 'llama-server exited with code 1',
      exit_code: 1,
      occurred_at: occurredAt,
    }
    await window.__TAURI_BROWSER_TEST__.emitEvent('runtime-service-status', {
      running: {},
      recovery: {
        'browser-test-instance': {
          phase: 'waiting',
          restart_attempts: 1,
          max_restart_attempts: 3,
          next_retry_at: occurredAt + 30,
          origin_failure: failure,
          last_failure: failure,
        },
      },
      previouslyManaged: [],
      lastError: failure.message,
    })
  }, { occurredAt })

  await expect(page.getByText('Recovery Incident')).toBeVisible()
  await expect(page.getByText('Automatic attempts: 1 / 3')).toBeVisible()
  await expect(page.getByText('Originating failure · Unexpected exit')).toBeVisible()
  await expect(page.getByText('llama-server exited with code 1', { exact: true })).toBeVisible()
  await expect(page.getByText('Occurred:', { exact: false })).toBeVisible()
  await expect(page.getByText('Exit code: 1', { exact: false })).toBeVisible()
  await expect(page.getByText('Next retry:', { exact: false })).toBeVisible()

  const cancelRecovery = page.getByRole('button', { name: 'Cancel Recovery', exact: true }).first()
  await expect(cancelRecovery).toBeEnabled()
  await cancelRecovery.click()
  await expect.poll(() => page.evaluate(() => (
    window.__TAURI_BROWSER_TEST__.calls
      .filter(call => call.command === 'stop_server')
      .map(call => call.payload)
  ))).toEqual([{ instanceId: 'browser-test-instance' }])
  await expect(page.getByText('Recovery Incident')).toBeHidden()
})

test('a failure with recovery disabled is presented as a plain runtime incident', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('lang', 'zh-CN')
    localStorage.setItem('lastTab', 'instances')
  })
  await page.goto('/')
  await expect(page.getByTitle('\u6545\u969C\u81EA\u6108')).toHaveAttribute('aria-checked', 'false')

  const occurredAt = Math.floor(Date.now() / 1000)
  await page.evaluate(async ({ occurredAt }) => {
    const failure = {
      kind: 'startup_failure',
      message: 'engine launch rejected',
      exit_code: null,
      occurred_at: occurredAt,
    }
    await window.__TAURI_BROWSER_TEST__.emitEvent('runtime-service-status', {
      running: {},
      recovery: {
        'browser-test-instance': {
          phase: 'failed',
          restart_attempts: 0,
          max_restart_attempts: 3,
          next_retry_at: null,
          origin_failure: failure,
          last_failure: failure,
        },
      },
      previouslyManaged: [],
      lastError: failure.message,
    })
  }, { occurredAt })

  await expect(page.getByText('\u8FD0\u884C\u6545\u969C')).toBeVisible()
  await expect(page.getByText('\u81EA\u6108\u4E8B\u4EF6')).toBeHidden()
  await expect(page.getByText(/\u81EA\u52A8\u5C1D\u8BD5/)).toBeHidden()
  await expect(page.getByRole('button', { name: '\u6E05\u9664\u6545\u969C\u8BB0\u5F55', exact: true })).toBeEnabled()
})
