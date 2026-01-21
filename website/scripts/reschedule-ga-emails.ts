#!/usr/bin/env tsx
/**
 * Reschedule GA Emails - Update scheduled emails to send immediately
 *
 * Usage:
 *   npx tsx scripts/reschedule-ga-emails.ts --dry-run    # Preview without updating
 *   npx tsx scripts/reschedule-ga-emails.ts              # Update all to send now
 *   npx tsx scripts/reschedule-ga-emails.ts --schedule "in 1 hour"  # Reschedule to specific time
 */

import 'dotenv/config'
import { Resend } from 'resend'
import * as readline from 'readline'

// Parse command line arguments
const args = process.argv.slice(2)
const getArg = (name: string): string | undefined => {
  const index = args.findIndex((a) => a.startsWith(`--${name}`))
  if (index === -1) return undefined
  const arg = args[index]
  if (arg.includes('=')) return arg.split('=')[1]
  return args[index + 1]
}
const hasFlag = (name: string): boolean => args.includes(`--${name}`)

const dryRun = hasFlag('dry-run')
const newSchedule = getArg('schedule') || 'in 1 min'

// Load environment variables
const resendKey = process.env.RESEND_API_KEY

if (!resendKey) {
  console.error('Error: RESEND_API_KEY must be set in .env')
  process.exit(1)
}

const resend = new Resend(resendKey)

// Helper to ask for confirmation
async function confirm(message: string): Promise<boolean> {
  const rl = readline.createInterface({
    input: process.stdin,
    output: process.stdout,
  })

  return new Promise((resolve) => {
    rl.question(`${message} (y/N): `, (answer) => {
      rl.close()
      resolve(answer.toLowerCase() === 'y' || answer.toLowerCase() === 'yes')
    })
  })
}

// Sleep helper
function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

// Main function
async function main() {
  console.log('\n🔄 Resend Scheduled Email Updater\n')

  if (dryRun) {
    console.log('📋 DRY RUN MODE - No emails will be updated\n')
  }

  // Fetch all emails and filter for scheduled ones
  console.log('Fetching emails from Resend...')

  const scheduledEmails: Array<{ id: string; to: string; subject: string; scheduled_at: string }> = []
  let cursor: string | undefined = undefined

  // Paginate through all emails
  let lastEmailId: string | undefined = undefined
  let hasMore = true
  let pageCount = 0

  while (hasMore) {
    const params: any = { limit: 100 }
    if (lastEmailId) {
      params.after = lastEmailId
    }

    const response = await resend.emails.list(params) as any

    if (response.error) {
      console.error('Error fetching emails:', response.error)
      process.exit(1)
    }

    const emails = response.data?.data || []
    hasMore = response.data?.has_more || false
    pageCount++

    // Filter for scheduled emails (have scheduled_at and last_event is 'scheduled')
    // This automatically skips already-processed emails since they won't have last_event='scheduled' anymore
    for (const email of emails) {
      if (email.scheduled_at && email.last_event === 'scheduled') {
        scheduledEmails.push({
          id: email.id,
          to: Array.isArray(email.to) ? email.to.join(', ') : email.to,
          subject: email.subject,
          scheduled_at: email.scheduled_at,
        })
      }
      lastEmailId = email.id
    }

    process.stdout.write(`  Page ${pageCount}: ${emails.length} emails fetched, ${scheduledEmails.length} scheduled so far\n`)

    // Stop if no more pages
    if (!hasMore || emails.length === 0) break

    // Delay to avoid rate limiting (2 req/sec limit)
    await sleep(600)
  }

  if (scheduledEmails.length === 0) {
    console.log('No scheduled emails found.')
    process.exit(0)
  }

  console.log(`Found ${scheduledEmails.length} scheduled email(s).\n`)

  // Show summary
  console.log('Scheduled emails:')
  scheduledEmails.slice(0, 10).forEach((email, i) => {
    console.log(`  ${i + 1}. ${email.to} - scheduled for ${email.scheduled_at}`)
  })
  if (scheduledEmails.length > 10) {
    console.log(`  ... and ${scheduledEmails.length - 10} more`)
  }
  console.log('')

  if (dryRun) {
    console.log('✅ Dry run complete. No emails updated.')
    return
  }

  // Confirm before updating
  const confirmed = await confirm(
    `\n⚠️  You are about to reschedule ${scheduledEmails.length} email(s) to "${newSchedule}". Continue?`
  )

  if (!confirmed) {
    console.log('Aborted.')
    process.exit(0)
  }

  // Update emails
  let updated = 0
  let failed = 0
  const errors: { id: string; to: string; error: string }[] = []

  console.log('\nUpdating emails...\n')

  for (const email of scheduledEmails) {
    try {
      const result = await resend.emails.update({
        id: email.id,
        scheduledAt: newSchedule,
      })

      if ((result as any).error) {
        throw new Error((result as any).error.message)
      }

      updated++
      process.stdout.write('.')
    } catch (err) {
      failed++
      errors.push({
        id: email.id,
        to: email.to,
        error: err instanceof Error ? err.message : String(err),
      })
      process.stdout.write('x')
    }

    // Delay to avoid rate limiting (2 req/sec limit)
    await sleep(600)
  }

  // Print summary
  console.log('\n\n' + '='.repeat(50))
  console.log('📊 Summary')
  console.log('='.repeat(50))
  console.log(`  ✅ Updated: ${updated}`)
  console.log(`  ❌ Failed: ${failed}`)
  console.log(`  📅 New schedule: ${newSchedule}`)

  if (errors.length > 0) {
    console.log('\nFailed updates:')
    errors.forEach(({ id, to, error }) => {
      console.log(`  - ${to} (${id}): ${error}`)
    })
  }

  console.log('\n🎉 Reschedule complete!')
}

main().catch((err) => {
  console.error('Fatal error:', err)
  process.exit(1)
})
