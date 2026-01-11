#!/usr/bin/env tsx
/**
 * Invite Script - Send invites to waitlist users
 *
 * Usage:
 *   pnpm invite --count 5           # Invite next 5 users
 *   pnpm invite --count 5 --dry-run # Preview without sending
 *   pnpm invite --start 5           # Start from position 5 (skip first 4)
 *   pnpm invite --email user@example.com  # Invite specific user by email
 *   pnpm invite --twitter handle          # Invite specific user by Twitter handle
 */

import 'dotenv/config'
import { createClient } from '@supabase/supabase-js'
import { Resend } from 'resend'
import { randomBytes } from 'crypto'
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

const count = parseInt(getArg('count') || '1', 10)
const start = parseInt(getArg('start') || '1', 10)
const dryRun = hasFlag('dry-run')
const targetEmail = getArg('email')
const targetTwitter = getArg('twitter')?.replace(/^@/, '') // Strip leading @ if present

// Load environment variables
const supabaseUrl = process.env.PUBLIC_SUPABASE_URL
const supabaseKey = process.env.PUBLIC_SUPABASE_ANON_KEY
const resendKey = process.env.RESEND_API_KEY
const siteUrl = process.env.SITE_URL || 'https://getconduit.sh'

if (!supabaseUrl || !supabaseKey) {
  console.error('Error: PUBLIC_SUPABASE_URL and PUBLIC_SUPABASE_ANON_KEY must be set')
  process.exit(1)
}

if (!resendKey && !dryRun) {
  console.error('Error: RESEND_API_KEY must be set (or use --dry-run)')
  process.exit(1)
}

const supabase = createClient(supabaseUrl, supabaseKey)
const resend = resendKey ? new Resend(resendKey) : null

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

// Generate invite token
function generateToken(): string {
  return randomBytes(32).toString('hex')
}

// Build invite email HTML (simplified version for CLI)
function buildInviteEmailHtml(inviteUrl: string): string {
  return `
<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
</head>
<body style="background-color: #0a0a0f; font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace; margin: 0; padding: 0;">
  <div style="padding: 40px 20px; max-width: 600px; margin: 0 auto;">
    <p style="color: #00ff88; font-size: 24px; font-weight: bold; text-align: center; margin: 0 0 30px 0; letter-spacing: 4px;">CONDUIT</p>
    <div style="background-color: #111118; padding: 30px; border-radius: 8px; border: 1px solid #2a2a3a;">
      <p style="color: #e0e0e8; font-size: 24px; font-weight: 600; margin: 0 0 20px 0; text-align: center;">You're Invited!</p>
      <p style="color: #a0a0b0; font-size: 14px; line-height: 1.6; margin: 0 0 24px 0; text-align: center;">
        Your spot on the Conduit waitlist has come up. You now have early access to run a team of AI agents in your terminal.
      </p>
      <div style="text-align: center; margin: 24px 0;">
        <a href="${inviteUrl}" style="background-color: #00ff88; color: #0a0a0f; padding: 14px 32px; border-radius: 6px; font-size: 14px; font-weight: bold; text-decoration: none; display: inline-block;">Accept Invite</a>
      </div>
      <hr style="border-color: #2a2a3a; border-width: 1px; margin: 24px 0;">
      <p style="color: #808090; font-size: 13px; line-height: 1.5; margin: 0 0 12px 0; text-align: center;">
        Click the button above to connect your GitHub account and get access to the private repository.
      </p>
      <p style="color: #ffaa00; font-size: 12px; text-align: center; margin: 0;">This invite expires in 7 days.</p>
    </div>
    <p style="color: #606070; font-size: 12px; text-align: center; margin-top: 30px;">
      Conduit - Run a team of AI agents in your terminal
    </p>
  </div>
</body>
</html>
`
}

async function main() {
  console.log('\n🚀 Conduit Invite Script\n')

  let entries: { id: string; email: string; twitter_handle: string | null; created_at: string }[] = []

  // Mode: specific user by email or twitter
  if (targetEmail || targetTwitter) {
    const searchField = targetEmail ? 'email' : 'twitter_handle'
    const searchValue = targetEmail || targetTwitter

    console.log(`Searching for user by ${targetEmail ? 'email' : 'Twitter'}: ${searchValue}\n`)

    const { data, error } = await supabase
      .from('waitlist')
      .select('id, email, twitter_handle, created_at, invited_at')
      .ilike(searchField, searchValue!)
      .limit(1)
      .single()

    if (error || !data) {
      console.error(`User not found with ${searchField} = ${searchValue}`)
      process.exit(1)
    }

    if (data.invited_at) {
      console.log(`⚠️  User already invited on ${new Date(data.invited_at).toLocaleString()}`)
      console.log(`   Email: ${data.email}`)
      console.log(`   Twitter: ${data.twitter_handle || '-'}`)
      console.log(`\nUse 'pnpm reset-invite --email ${data.email} --send' to resend.`)
      process.exit(0)
    }

    entries = [data]
  } else {
    // Mode: batch invite
    console.log(`Settings:`)
    console.log(`  Count: ${count}`)
    console.log(`  Start position: ${start}`)
    console.log(`  Dry run: ${dryRun ? 'Yes' : 'No'}\n`)

    const { data, error } = await supabase
      .from('waitlist')
      .select('id, email, twitter_handle, created_at')
      .is('invited_at', null)
      .order('created_at', { ascending: true })
      .range(start - 1, start - 1 + count - 1)

    if (error) {
      console.error('Error fetching waitlist:', error.message)
      process.exit(1)
    }

    entries = data || []
  }

  if (entries.length === 0) {
    console.log('No uninvited users found.')
    process.exit(0)
  }

  // Show preview
  console.log(`Found ${entries.length} user(s) to invite:\n`)
  if (targetEmail || targetTwitter) {
    // Simple display for single user
    const entry = entries[0]
    console.log(`  Email:   ${entry.email}`)
    console.log(`  Twitter: ${entry.twitter_handle || '-'}`)
    console.log()
  } else {
    // Table display for batch
    console.log('┌───────────────────────────────────────────────────────────┐')
    console.log('│ #   │ Email                          │ Twitter            │')
    console.log('├───────────────────────────────────────────────────────────┤')
    entries.forEach((entry, i) => {
      const email = entry.email.padEnd(30).slice(0, 30)
      const twitter = (entry.twitter_handle || '-').padEnd(18).slice(0, 18)
      console.log(`│ ${String(start + i).padStart(3)} │ ${email} │ ${twitter} │`)
    })
    console.log('└───────────────────────────────────────────────────────────┘\n')
  }

  if (dryRun) {
    console.log('Dry run mode - no invites will be sent.')
    process.exit(0)
  }

  // Confirm
  const confirmed = await confirm(`Send ${entries.length} invite(s)?`)
  if (!confirmed) {
    console.log('Aborted.')
    process.exit(0)
  }

  console.log('\nSending invites...\n')

  let successCount = 0
  let failCount = 0

  for (const entry of entries) {
    const token = generateToken()
    const inviteUrl = `${siteUrl}/invite/${token}`
    const expiresAt = new Date(Date.now() + 7 * 24 * 60 * 60 * 1000).toISOString()

    try {
      // Insert invite token
      const { error: tokenError } = await supabase.from('invite_tokens').insert({
        token,
        waitlist_id: entry.id,
        expires_at: expiresAt,
      })

      if (tokenError) {
        console.error(`  ✗ ${entry.email}: Failed to create token - ${tokenError.message}`)
        failCount++
        continue
      }

      // Update waitlist invited_at
      const { error: updateError } = await supabase
        .from('waitlist')
        .update({ invited_at: new Date().toISOString() })
        .eq('id', entry.id)

      if (updateError) {
        console.error(`  ✗ ${entry.email}: Failed to update waitlist - ${updateError.message}`)
        failCount++
        continue
      }

      // Send email
      if (resend) {
        const { error: emailError } = await resend.emails.send({
          from: 'Felipe Coury <felipe@getconduit.sh>',
          to: entry.email,
          subject: "You're invited to access Conduit",
          html: buildInviteEmailHtml(inviteUrl),
        })

        if (emailError) {
          console.error(`  ✗ ${entry.email}: Failed to send email - ${emailError.message}`)
          failCount++
          continue
        }
      }

      console.log(`  ✓ ${entry.email}`)
      successCount++
    } catch (err) {
      console.error(`  ✗ ${entry.email}: ${err}`)
      failCount++
    }
  }

  console.log('\n────────────────────────────────────────')
  console.log(`Summary: ${successCount} sent, ${failCount} failed`)
  console.log('────────────────────────────────────────\n')
}

main().catch((err) => {
  console.error('Fatal error:', err)
  process.exit(1)
})
