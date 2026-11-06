# Hub text in English. This file is the fallback for every language: a
# message that another language lacks is shown from here.
#
# To add a message: write `id = text` on one line (or an indented block),
# add the same id to every other .ftl file, and call `t("id")` with the id
# as a literal. A message never starts or ends with a space; Fluent removes
# it. Pass codes and ids as strings, because a number argument is shown
# with digit grouping.

language-picker-label = Language
language-not-saved = This browser does not let the Hub save the language, so it stays as it is.

age-moments = moments ago

money-input-invalid = "{ $input }" is not an amount like { $example } for this currency

# Errors. `$detail` is the service's own English text.
error-service-unavailable = { $detail }
error-service-timeout = { $detail }
error-permission = { $detail }
error-quota = { $detail }
error-not-signed-in = { $detail }
error-not-owner = { $detail }
error-no-owner = { $detail }
error-not-local = this installation refused a request that did not come from you
error-method-not-found = { $detail }
error-invalid-input = { $detail }
error-internal = { $detail }
error-service = { $detail }
error-network = Could not reach this installation: { $detail }
error-http = HTTP error { $code }

# Failures shown beside an action. `$detail` is already worded by the Hub.
login-failed = Sign-in failed: { $detail }
directory-search-failed = The search failed: { $detail }. Press Search to try again.
directory-search-not-started = This installation could not start the search. Press Search to try again.
messages-quote-expired = This quote has expired. Ask for a new one.
messages-accept-failed = Could not accept: { $detail }
contacts-upsert-failed = Upsert failed: { $detail }
safety-block-failed = Block failed: { $detail }
safety-report-failed = Report failed: { $detail }
safety-save-failed = Save failed: { $detail }
backup-export-failed = Export failed: { $detail }
backup-import-failed = Import failed: { $detail }
profile-error = Error: { $detail }

# Sentences that the Rust side also holds (crates/roym_core). A test reads
# this file and checks that each one appears here word for word, on one
# line. Do not wrap them or change a word without changing the Rust side.
payment-notice = This records what each side says about the payment. Roym does not see the money move and cannot confirm that it did.
progress-notice = This status comes from the system of the person who wrote the quote. It is not a signed statement by either person.
payment-claimed = The payer says they paid.
payment-acknowledged = The payee confirms they received the payment.
fulfilment-claimed = The person doing the work says it is done.
fulfilment-acknowledged = The person receiving the work confirms it is done.
track-unconfirmed = No confirmation was recorded before the window closed.
one-payment-notice = This quote is paid in one payment. Deposits and part payments are not supported.

group-owner-can-read-notice = The owner of this group makes and shares the group's key, so the owner can read every message sent while they own it. Adding or removing a member is shown to everyone in the group.
group-key-trust-notice = Each member's messages are signed with a key this installation first saw when they joined. That is a weaker check than a signed record, so messages here are never marked as verified.
group-delivery-notice = "Not yet delivered to every member" means at least one member has not received it directly. Members also pass messages to each other, so a member may still receive it later.
group-join-boundary-notice = You can read messages sent after you joined. Messages from before you joined are not shared with you.
group-removed-notice = You were removed from this group. You can still read what you received before. You cannot read or send new messages.
group-restored-notice = This group's history was restored from a backup. This installation is not a member, so it cannot send or receive new messages here. Ask the owner to add your new address.
group-add-unreachable = Could not reach this person to add them. Someone you have not talked to before must be online when you add them.
group-hidden-notice = A hidden group is not shown. New messages are held for up to 30 days, and you stay a member until the owner removes you.
group-transcript-check-notice = Members who see the same code hold the same messages in the same order.
group-cards-not-allowed = Cards are sent only in a 1:1 conversation.

# Group lines
group-session-ended-notice = Your session ended, for example because this installation restarted. Log in again.
group-delivery-pending = Not yet delivered to every member
group-delivery-delivered = Delivered to every member
group-delivery-failed = Not delivered to every member
group-event-created = { $author } created the group
group-event-added = { $author } added { $subject }
group-event-removed = { $author } removed { $subject }
group-event-updated = { $author } updated the group

# Cards. A label that is followed by a value of another kind (a link, a
# time) has no `$value`: the card draws the value after it.
card-unknown-title = Unknown Card ({ $type } v{ $version })
card-unknown-body = This client does not understand this card type or version.
card-refused-title = Unverified { $type } card (v{ $version })
card-refused-notice = This node received a card of this type, but could not verify it. It is shown so you know it was sent, but cannot be accepted or acted upon.
card-refused-reason = Reason: { $reason }

# The terms shared by the quote and the agreement receipt
terms-scope = Scope: { $value }
terms-total = Total: { $amount }
terms-tax = Tax: { $amount }
terms-fees = Fees: { $amount }
terms-timing-before = Payment before the work
terms-timing-after = Payment after the work
terms-payee-label = Payee, as agreed in this quote:
terms-schedule-label = Schedule:
terms-range-separator = { " - " }
terms-location = Location: { $where }
terms-address = Address given in this quote: { $address }
terms-cancellation = Cancellation terms: { $value }
terms-refund = Refund terms: { $value }
terms-dispute-label = Dispute path:

quote-title = Quote
quote-expiry-label = Expires:
quote-expires-expired = Expires: { $date } (expired)
quote-expired-notice = This quote expired on { $date }. Ask for a new one.

receipt-title = Agreement Receipt
receipt-pair-complete = Both parties have signed these terms.
receipt-pair-half-recipient = Only the person the quote was sent to has signed so far.
receipt-pair-half-author = Only the person who wrote the quote has signed so far.
receipt-role-author = Signed by: the person who wrote the quote
receipt-role-recipient = Signed by: the person the quote was sent to
receipt-quote-to = Quote to: { $did }
receipt-quote-by = Quote by: { $did }
receipt-quote-record = Quote record: { $id }
receipt-expiry-label = Quote expiry:

request-title = Request
request-categories = Categories: { $categories }
request-listing = Listing: { $id }
request-window-label = Window:
request-notice = Notice: { $value }

progress-title = Booking Progress
progress-status = Status: { $state }
progress-state-scheduled = Scheduled
progress-state-in-progress = In progress
progress-state-completed = Completed
progress-state-cancelled = Cancelled
progress-state-conflict = Conflict
progress-state-ended-unconfirmed = Ended, unconfirmed
progress-conflict-slot-taken = Another booking took this slot first.
progress-conflict-slot-unavailable = The listing owner removed this slot.
progress-cancel-reason = Cancellation reason: { $reason }

payment-request-title = Payment Request
payment-ack-title = Payment Acknowledgement
payment-amount = Amount: { $amount }
payment-note = Note: { $note }
payment-observed-label = Observed:
payment-method = Method: { $method }
payment-reference = Reference: { $reference }
payment-corrected = This is a corrected statement.

fulfilment-title = Fulfilment Receipt

# Places and audiences of a listing
place-editor-at-giver = At my place
place-editor-at-receiver = At their place
place-remote = Remote
place-card-at-giver = At the quote author's place
place-card-at-receiver = At the counterparty's place
audience-anyone = Anyone
audience-members = Members
audience-referral = Referral
audience-past-counterparties = People I have worked with before

# The Hub shell: title, session bar, tabs, sign-in box and card gallery
hub-title = Roym Hub
session-logged-in = Logged in: { $did } ({ $auth })
session-logout = Log out
session-anonymous = Not logged in
tab-error = Could not load this tab: { $detail }
tab-profile = Profile
tab-contacts = Contacts
tab-messages = Messages
tab-groups = Groups
tab-listings = Listings
tab-directory = Directory
tab-memberships = Memberships
tab-synorg = SynOrg
tab-safety = Safety
tab-backup = Backup
tab-components = Components
login-title = Sign in
login-auth-unreachable = Could not reach the auth service on this node.
login-no-method = This node has no browser login method enabled.
login-intro = Load a session key file produced by `roymctl session delegate --as <name> --registry-url <registry>`. The key stays in this browser and cannot be read back by page code.
login-with-stored = Sign in with this browser's key
login-signing-in = Signing in...
login-stored-gone = The stored key is gone; load a session key file.
login-load-file = Load session key file
gallery-home-title = Roym Hub Home
gallery-did = Authenticated DID: { $did }
gallery-contacting = Contacting profile service...
gallery-ping-response = Profile ping response: { $response }
gallery-ping-failed = Profile ping failed: HTTP { $status }
gallery-ping-error = Profile ping error: { $error }
gallery-heading = Card Gallery (Sample Cards)

# Shown where a time from another node is not a date
date-invalid = Invalid Date

# Lines of the Directory and SynOrg screens that show a time
directory-signed-age = Signed { $age }
synorg-credential-expires = expires { $date }
synorg-until = until { $date }
synorg-until-lifted = until lifted

## Shared

error-unexpected = { $detail }
error-no-detail = RPC error
