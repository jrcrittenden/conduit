#!/usr/bin/env tsx
/**
 * Reset Invite - Reset a user's invite status for testing
 *
 * Usage:
 *   pnpm reset-invite --email user@example.com
 *   pnpm reset-invite --email user@example.com --send   # Also send new invite
 */

import 'dotenv/config'
import { createClient } from '@supabase/supabase-js'
import { Resend } from 'resend'
import { randomBytes } from 'crypto'

const args = process.argv.slice(2)
const getArg = (name: string): string | undefined => {
  const index = args.findIndex((a) => a.startsWith(`--${name}`))
  if (index === -1) return undefined
  const arg = args[index]
  if (arg.includes('=')) return arg.split('=')[1]
  return args[index + 1]
}
const hasFlag = (name: string): boolean => args.includes(`--${name}`)

const email = getArg('email')
const sendInvite = hasFlag('send')

if (!email) {
  console.error('Usage: pnpm reset-invite --email <email> [--send]')
  process.exit(1)
}

const supabaseUrl = process.env.PUBLIC_SUPABASE_URL
const supabaseKey = process.env.PUBLIC_SUPABASE_ANON_KEY

if (!supabaseUrl || !supabaseKey) {
  console.error('Error: PUBLIC_SUPABASE_URL and PUBLIC_SUPABASE_ANON_KEY must be set')
  process.exit(1)
}

const resendKey = process.env.RESEND_API_KEY
const siteUrl = process.env.SITE_URL || 'https://getconduit.sh'

if (sendInvite && !resendKey) {
  console.error('Error: RESEND_API_KEY must be set to send invites')
  process.exit(1)
}

const supabase = createClient(supabaseUrl, supabaseKey)
const resend = resendKey ? new Resend(resendKey) : null

function generateToken(): string {
  return randomBytes(32).toString('hex')
}

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
  console.log(`\n🔄 Resetting invite for ${email}...\n`)

  // Find the waitlist entry
  const { data: waitlist, error: findError } = await supabase
    .from('waitlist')
    .select('id, email, invited_at, accepted_at, github_username')
    .eq('email', email)
    .single()

  if (findError || !waitlist) {
    console.error('✗ User not found:', findError?.message || 'No matching email')
    process.exit(1)
  }

  console.log('Current status:')
  console.log(`  Email: ${waitlist.email}`)
  console.log(`  Invited at: ${waitlist.invited_at || 'Never'}`)
  console.log(`  Accepted at: ${waitlist.accepted_at || 'Never'}`)
  console.log(`  GitHub: ${waitlist.github_username || 'N/A'}\n`)

  // Delete any existing invite tokens
  const { error: deleteTokenError } = await supabase
    .from('invite_tokens')
    .delete()
    .eq('waitlist_id', waitlist.id)

  if (deleteTokenError) {
    console.error('✗ Failed to delete tokens:', deleteTokenError.message)
  } else {
    console.log('✓ Deleted existing invite tokens')
  }

  // Reset the waitlist entry
  const { error: resetError } = await supabase
    .from('waitlist')
    .update({
      invited_at: null,
      accepted_at: null,
      github_username: null,
    })
    .eq('id', waitlist.id)

  if (resetError) {
    console.error('✗ Failed to reset waitlist:', resetError.message)
    process.exit(1)
  }

  console.log('✓ Reset waitlist entry (invited_at, accepted_at, github_username)\n')

  if (sendInvite && resend) {
    // Create new invite token
    const token = generateToken()
    const inviteUrl = `${siteUrl}/invite/${token}`
    const expiresAt = new Date(Date.now() + 7 * 24 * 60 * 60 * 1000).toISOString()

    const { error: tokenError } = await supabase.from('invite_tokens').insert({
      token,
      waitlist_id: waitlist.id,
      expires_at: expiresAt,
    })

    if (tokenError) {
      console.error('✗ Failed to create invite token:', tokenError.message)
      process.exit(1)
    }

    // Update waitlist invited_at
    const { error: updateError } = await supabase
      .from('waitlist')
      .update({ invited_at: new Date().toISOString() })
      .eq('id', waitlist.id)

    if (updateError) {
      console.error('✗ Failed to update invited_at:', updateError.message)
      process.exit(1)
    }

    // Send email
    const { error: emailError } = await resend.emails.send({
      from: 'Felipe Coury <felipe@getconduit.sh>',
      to: email!,
      subject: "You're invited to access Conduit",
      html: buildInviteEmailHtml(inviteUrl),
    })

    if (emailError) {
      console.error('✗ Failed to send email:', emailError.message)
      process.exit(1)
    }

    console.log('✓ Created new invite token')
    console.log('✓ Sent invite email\n')
    console.log(`Invite URL: ${inviteUrl}\n`)
  } else {
    console.log('Done! You can now send a fresh invite with:')
    console.log(`  pnpm invite --count 1\n`)
  }
}

main().catch((err) => {
  console.error('Fatal error:', err)
  process.exit(1)
})
