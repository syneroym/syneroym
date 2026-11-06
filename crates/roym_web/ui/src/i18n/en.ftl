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

## Sign-in

login-error-odd-hex = odd-length hex string
login-error-key-length = temporary secret key must be 32 bytes
login-error-challenge = challenge failed: HTTP { $status }
login-error-no-assertion = auth service did not return an assertion to sign
login-error-http = HTTP { $status }
login-error-service = { $detail }
login-error-methods = methods failed: HTTP { $status }

common-saved = Saved.
common-save-failed = Save failed: { $detail }
common-remove = Remove

## Profile

profile-title = Profile
profile-display-name = Display Name:
profile-conversation-address = Conversation Address:
profile-save = Save Profile
profile-saving = Saving...
profile-saved = Saved record { $record } (issuer: { $issuer })

## Contacts

contacts-title = Contacts
contacts-load-failed = Could not load contacts.
contacts-add-heading = Add / Import Contact
contacts-did-placeholder = Person DID
contacts-address-placeholder = Conversation Address (required if no profile envelope)
contacts-envelope-placeholder = Paste Profile Envelope JSON (optional)
contacts-upsert-button = Upsert Contact
contacts-envelope-invalid = Profile envelope is not valid JSON

## Set-up

setup-title = Signing Certificate Required
setup-intro = This installation has no active record-signing certificate enrolled. To enrol, run `roymctl roym enrol-signing --master <identity> --registry-url <registry>` from your terminal. The browser cannot do this because it holds no master key.
setup-missing = Missing signing certificates for: { $services }
setup-check-again = Check again

## Backup

backup-title = App Data Backup & Restore
backup-intro = This exports and imports application data. It never exports or touches your identity secret key.
backup-separate-note = Each bundle below is signed by you when it is exported, and is refused on import if it was changed. An encrypted backup of everything, including your identity, is made with `roymctl roym backup create`.
backup-bundle-profile = Profile, contacts, and safety records
backup-bundle-conversation = Conversations and messages
backup-bundle-catalog = Listings and availability
backup-bundle-transaction = Requests, quotes, and agreements
backup-bundle-directory = This installation's SynOrg directory, if it runs one
backup-export = Export
backup-import = Import
backup-reading = Reading file...
backup-importing = Importing...
backup-imported = Imported: { $count } record(s).
backup-imported-unknown = Imported: bundle record(s).

## Safety

safety-title = Safety & Blocklist
safety-policy-unavailable = Could not load this node's safety policy.
safety-block-placeholder = Person DID or Address to block
safety-block-button = Block
safety-blocked-line = Blocked: { $who }
safety-block-list-failed = Could not load block list.
safety-report-heading = Report a person, listing, or message
safety-kind-person = person
safety-kind-listing = listing
safety-kind-message = message
safety-subject-placeholder = Subject DID or id
safety-category-impersonation = impersonation
safety-category-fraud = fraud
safety-category-harassment = harassment
safety-category-unsafe-service = unsafe-service
safety-category-illegal-content = illegal-content
safety-details-placeholder = What happened (optional)
safety-file-report = File report
safety-report-row = { $category } - { $kind } { $subject } [{ $status }]
safety-status-open = open
safety-status-withdrawn = withdrawn
safety-withdraw = Withdraw
safety-reports-load-failed = Could not load reports.
safety-subject-required = A subject is required.
safety-recorded = Recorded { $id } ({ $status })
safety-limit-heading = First-contact rate limit
safety-limit-intro = How many strangers may start a conversation with you inside one window. An ongoing conversation is never limited.
safety-window-placeholder = window (seconds)
safety-max-placeholder = max new contacts per window
safety-save-limit = Save limit
safety-limit-load-failed = Could not load the current limit.
safety-whole-numbers = Both values must be whole numbers.

## Listings

listing-screen-title = Listings
listing-your-heading = Your listings
listing-load-failed = Could not load listings: { $detail }
listing-empty = No listings yet.
listing-version = v{ $count }
listing-history-button = History
listing-history-line = #{ $n } { $id } (supersedes { $previous })
listing-history-first = #{ $n } { $id } (first version)
listing-history-failed = History failed: { $detail }
listing-withdraw = Withdraw
listing-withdraw-failed = Withdraw failed: { $detail }
listing-publish-heading = Publish to a directory
listing-publish-button = Publish here
listing-published = Published.
listing-not-published = Not published: { $reason }
listing-editor-heading = New / edit a listing
listing-save-button = Save listing
listing-saved = Saved { $id } as version { $version } ({ $record })
listing-availability-heading = Availability (for slot bookings)
listing-add-slot = Add slot
listing-slot-line = { $start } - { $end } (x{ $capacity })
listing-slots-load-failed = Could not load slots: { $detail }
listing-verify-heading = Check a listing someone sent me
listing-verify-placeholder = Paste a listing envelope JSON
listing-verify-button = Check
listing-verify-bad-json = That is not valid JSON.
listing-verified = Verified.
listing-verify-issuer = Issuer: { $issuer }
listing-verify-address = Conversation address: { $address }
listing-verify-status = Status: { $status }
listing-not-verified = Not verified -- treat as unknown.
listing-check-failed = Check failed: { $detail }
listing-label-title = Title
listing-label-slug = Slug
listing-label-summary = Summary
listing-label-categories = Categories
listing-label-address = Conversation address
listing-label-status = Status
listing-label-mode = Mode
listing-label-lead-time = Lead time (s)
listing-label-cancellation-window = Cancellation window (s)
listing-label-max-per-booking = Max per booking
listing-label-currency = Currency
listing-label-model = Model
listing-label-amount = Amount
listing-label-fees = Fees
listing-label-methods = Methods
listing-label-payee = Payee
listing-label-tax-included = tax included
listing-label-unit = Unit
listing-label-pack-size = Pack size
listing-label-condition = Condition
listing-label-sku = SKU
listing-label-duration = Duration (s)
listing-label-includes = Includes
listing-label-excludes = Excludes
listing-label-prerequisites = Prerequisites
listing-label-where = Where
listing-label-disclosure = Address disclosure
listing-label-service-area = Service area
listing-label-open-to = Open to
listing-label-group-did = Group DID
listing-label-fulfilment-receipt = issues a fulfilment receipt
listing-label-warranty = Warranty (s)
listing-label-retention = Retention (s)
listing-label-start = Start
listing-label-end = End
listing-label-capacity = Capacity
listing-placeholder-slug = slug (optional; derived from the title)
listing-placeholder-title = title
listing-placeholder-summary = summary
listing-placeholder-categories = categories, comma separated
listing-placeholder-address = conversation address (optional; taken from your profile)
listing-placeholder-lead-time = lead time (seconds)
listing-placeholder-cancellation-window = cancellation window (seconds)
listing-placeholder-max-per-booking = max per booking
listing-placeholder-currency = currency (ISO-4217, e.g. EUR)
listing-placeholder-amount = amount, e.g. 35.50
listing-placeholder-fees = fees, e.g. 2.50 (optional)
listing-placeholder-methods = methods, comma separated
listing-placeholder-payee = payee (informational only)
listing-placeholder-unit = unit, e.g. item
listing-placeholder-pack-size = pack size
listing-placeholder-sku = SKU (optional)
listing-placeholder-duration = duration (seconds)
listing-placeholder-includes = includes, comma separated
listing-placeholder-excludes = excludes, comma separated
listing-placeholder-prerequisites = prerequisites, comma separated
listing-placeholder-area = service area label (optional)
listing-placeholder-group-did = group DID (required for members)
listing-placeholder-warranty = warranty (seconds)
listing-placeholder-retention = retention (seconds)
listing-placeholder-capacity = capacity
listing-placeholder-directory-did = Roym Directory service DID
listing-status-active = active
listing-status-draft = draft
listing-status-withdrawn = withdrawn
listing-mode-enquiry = enquiry
listing-mode-slots = slots
listing-mode-order = order
listing-model-per-hour = per-hour
listing-model-fixed = fixed
listing-model-per-unit = per-unit
listing-model-quote-only = quote-only
listing-condition-new = new
listing-condition-used = used
listing-condition-refurbished = refurbished
listing-disclosure-on-agreement = on-agreement
listing-disclosure-public = public
listing-block-booking = booking
listing-block-payment = payment
listing-block-product = product
listing-block-service = service
listing-block-location = location
listing-block-relationship = relationship
listing-block-service-record = service_record
listing-field-lead-time = lead time
listing-field-cancellation-window = cancellation window
listing-field-max-per-booking = max per booking
listing-field-pack-size = pack size
listing-field-duration = duration
listing-field-warranty = warranty
listing-field-retention = retention
listing-field-capacity = capacity
listing-error-coordinate = "{ $input }" is not a coordinate in degrees
listing-error-whole = { $field } must be a whole number of seconds or units
listing-error-required = { $field } is required
listing-error-currency = unknown currency '{ $currency }'
listing-error-amount-required = an amount is required unless the model is quote-only
listing-error-quote-only-amount = a quote-only listing must not carry an amount
listing-error-members-group = a members-only listing needs the group DID
listing-error-title-required = a title is required
listing-error-slot-times = a slot needs a start and an end time
listing-error-slot-order = a slot's end must be after its start
listing-error-slot-capacity = a slot needs a capacity of at least 1
