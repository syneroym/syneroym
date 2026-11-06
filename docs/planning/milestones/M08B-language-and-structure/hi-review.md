# M08B slice B1b: Hindi text review

*Reader: the project owner. This is a working file. It is deleted before the PR merges.*

This file shows every message of the Hub in English and Hindi, side by side. An agent wrote the Hindi text. Please read it and mark each row you want changed. Write the new Hindi text in your reply, or tell me the rule ("always say X, not Y").

Please read the 57 rows marked **legal or safety** first, and slowly. They are the notices a person reads before they sign, delete or share something, and the words of quotes and agreements. A wrong word there can change what a person agrees to.

When you are done, the fix session changes `hi.ftl`, sets the first comment of `hi.ftl` to `# owner review: done <date>`, and deletes this file. The PR does not merge before that.

How to read the table:

- `{ $name }` is a value that the Hub puts in. Keep it in the Hindi text.
- A message with `->` and lines that start with `[` or `*[` picks a different text for a different value (for example with or without a reason). The table shows it on one line.
- Product names (Roym, SynOrg, DID) and commands stay in Latin letters.
- "screen" is the part of the Hub where the message shows. For the older messages it is the first words of the comment above their group in the file.

Total: 644 messages (57 with legal or safety weight, 587 others).

## Legal or safety rows (57)

| id | English | Hindi | screen | weight |
| --- | --- | --- | --- | --- |
| `messages-quote-expired` | This quote has expired. Ask for a new one. | यह कोटेशन समाप्त हो गया है। नया कोटेशन माँगें। | Failures shown beside an action. … | legal or safety |
| `messages-accept-failed` | Could not accept: { $detail } | स्वीकार नहीं हो सका: { $detail } | Failures shown beside an action. … | legal or safety |
| `payment-notice` | This records what each side says about the payment. Roym does not see the money move and cannot confirm that it did. | यह उस बात को दर्ज करता है जो हर पक्ष भुगतान के बारे में कहता है। Roym पैसे का लेन-देन नहीं देखता और यह पुष्टि नहीं कर सकता कि भुगतान हुआ। | Sentences that the Rust side also… | legal or safety |
| `progress-notice` | This status comes from the system of the person who wrote the quote. It is not a signed statement by either person. | यह स्थिति उस व्यक्ति के सिस्टम से आती है जिसने कोटेशन लिखा। यह किसी भी व्यक्ति का हस्ताक्षरित बयान नहीं है। | Sentences that the Rust side also… | legal or safety |
| `one-payment-notice` | This quote is paid in one payment. Deposits and part payments are not supported. | इस कोटेशन का भुगतान एक ही बार में होता है। अग्रिम और किस्तों में भुगतान की सुविधा नहीं है। | Sentences that the Rust side also… | legal or safety |
| `group-owner-can-read-notice` | The owner of this group makes and shares the group's key, so the owner can read every message sent while they own it. Adding or removing a member is shown to everyone in the group. | इस समूह का मालिक समूह की कुंजी बनाता और साझा करता है, इसलिए जब तक वह मालिक है, वह भेजा गया हर संदेश पढ़ सकता है। किसी सदस्य को जोड़ना या हटाना समूह में सभी को दिखता है। |  | legal or safety |
| `group-key-trust-notice` | Each member's messages are signed with a key this installation first saw when they joined. That is a weaker check than a signed record, so messages here are never marked as verified. | हर सदस्य के संदेश उस कुंजी से हस्ताक्षरित होते हैं जो इस इंस्टॉलेशन ने सदस्य के जुड़ते समय पहली बार देखी थी। यह हस्ताक्षरित रिकॉर्ड से कमज़ोर जाँच है, इसलिए यहाँ के संदेश कभी "सत्यापित" नहीं माने जाते। |  | legal or safety |
| `group-delivery-notice` | "Not yet delivered to every member" means at least one member has not received it directly. Members also pass messages to each other, so a member may still receive it later. | "अभी हर सदस्य तक नहीं पहुँचा" का मतलब है कि कम से कम एक सदस्य को यह सीधे नहीं मिला। सदस्य आपस में भी संदेश भेजते हैं, इसलिए हो सकता है कि वह इसे बाद में पा ले। |  | legal or safety |
| `group-join-boundary-notice` | You can read messages sent after you joined. Messages from before you joined are not shared with you. | आप जुड़ने के बाद भेजे गए संदेश पढ़ सकते हैं। जुड़ने से पहले के संदेश आपके साथ साझा नहीं किए जाते। |  | legal or safety |
| `group-removed-notice` | You were removed from this group. You can still read what you received before. You cannot read or send new messages. | आपको इस समूह से हटा दिया गया है। जो संदेश आपको पहले मिले थे, उन्हें आप अब भी पढ़ सकते हैं। आप नए संदेश न पढ़ सकते हैं, न भेज सकते हैं। |  | legal or safety |
| `group-restored-notice` | This group's history was restored from a backup. This installation is not a member, so it cannot send or receive new messages here. Ask the owner to add your new address. | इस समूह का इतिहास बैकअप से वापस लाया गया है। यह इंस्टॉलेशन अब सदस्य नहीं है, इसलिए यहाँ नए संदेश न भेज सकता है, न पा सकता है। मालिक से कहें कि आपका नया पता जोड़ दे। |  | legal or safety |
| `group-hidden-notice` | A hidden group is not shown. New messages are held for up to 30 days, and you stay a member until the owner removes you. | छिपाया हुआ समूह दिखाया नहीं जाता। नए संदेश 30 दिन तक रखे जाते हैं, और जब तक मालिक आपको हटा न दे, आप सदस्य बने रहते हैं। |  | legal or safety |
| `group-delivery-pending` | Not yet delivered to every member | अभी हर सदस्य तक नहीं पहुँचा | Group lines | legal or safety |
| `group-delivery-delivered` | Delivered to every member | हर सदस्य तक पहुँच गया | Group lines | legal or safety |
| `group-delivery-failed` | Not delivered to every member | हर सदस्य तक नहीं पहुँचा | Group lines | legal or safety |
| `terms-scope` | Scope: { $value } | काम का ब्योरा: { $value } | The terms shared by the quote and… | legal or safety |
| `terms-total` | Total: { $amount } | कुल: { $amount } | The terms shared by the quote and… | legal or safety |
| `terms-tax` | Tax: { $amount } | कर: { $amount } | The terms shared by the quote and… | legal or safety |
| `terms-fees` | Fees: { $amount } | शुल्क: { $amount } | The terms shared by the quote and… | legal or safety |
| `terms-timing-before` | Payment before the work | काम से पहले भुगतान | The terms shared by the quote and… | legal or safety |
| `terms-timing-after` | Payment after the work | काम के बाद भुगतान | The terms shared by the quote and… | legal or safety |
| `terms-payee-label` | Payee, as agreed in this quote: | भुगतान पाने वाला, जैसा इस कोटेशन में तय हुआ: | The terms shared by the quote and… | legal or safety |
| `terms-schedule-label` | Schedule: | समय: | The terms shared by the quote and… | legal or safety |
| `terms-range-separator` | { " - " } | { " - " } | The terms shared by the quote and… | legal or safety |
| `terms-location` | Location: { $where } | जगह: { $where } | The terms shared by the quote and… | legal or safety |
| `terms-address` | Address given in this quote: { $address } | इस कोटेशन में दिया गया पता: { $address } | The terms shared by the quote and… | legal or safety |
| `terms-cancellation` | Cancellation terms: { $value } | रद्द करने की शर्तें: { $value } | The terms shared by the quote and… | legal or safety |
| `terms-refund` | Refund terms: { $value } | वापसी की शर्तें: { $value } | The terms shared by the quote and… | legal or safety |
| `terms-dispute-label` | Dispute path: | विवाद सुलझाने का तरीका: | The terms shared by the quote and… | legal or safety |
| `quote-title` | Quote | कोटेशन |  | legal or safety |
| `quote-expiry-label` | Expires: | वैधता खत्म होने का समय: |  | legal or safety |
| `quote-expires-expired` | Expires: { $date } (expired) | वैधता खत्म होने का समय: { $date } (खत्म हो चुकी) |  | legal or safety |
| `quote-expired-notice` | This quote expired on { $date }. Ask for a new one. | यह कोटेशन { $date } को खत्म हो गया। नया कोटेशन माँगें। |  | legal or safety |
| `receipt-title` | Agreement Receipt | समझौते की रसीद |  | legal or safety |
| `receipt-pair-complete` | Both parties have signed these terms. | दोनों पक्षों ने इन शर्तों पर हस्ताक्षर कर दिए हैं। |  | legal or safety |
| `receipt-pair-half-recipient` | Only the person the quote was sent to has signed so far. | अभी केवल उस व्यक्ति ने हस्ताक्षर किए हैं जिसे कोटेशन भेजा गया था। |  | legal or safety |
| `receipt-pair-half-author` | Only the person who wrote the quote has signed so far. | अभी केवल कोटेशन लिखने वाले व्यक्ति ने हस्ताक्षर किए हैं। |  | legal or safety |
| `receipt-role-author` | Signed by: the person who wrote the quote | हस्ताक्षर करने वाला: कोटेशन लिखने वाला व्यक्ति |  | legal or safety |
| `receipt-role-recipient` | Signed by: the person the quote was sent to | हस्ताक्षर करने वाला: जिसे कोटेशन भेजा गया वह व्यक्ति |  | legal or safety |
| `receipt-quote-to` | Quote to: { $did } | कोटेशन किसे: { $did } |  | legal or safety |
| `receipt-quote-by` | Quote by: { $did } | कोटेशन किसका: { $did } |  | legal or safety |
| `receipt-quote-record` | Quote record: { $id } | कोटेशन रिकॉर्ड: { $id } |  | legal or safety |
| `receipt-expiry-label` | Quote expiry: | कोटेशन की वैधता: |  | legal or safety |
| `request-data-use-notice` | This request is signed by you and sent to the person or business you chose. They keep a copy. It carries the area you gave, not your exact address; an address is disclosed only inside a quote you accept. | यह अनुरोध आपके हस्ताक्षर से बनता है और आपके चुने हुए व्यक्ति या व्यवसाय को भेजा जाता है। वे इसकी एक प्रति रखते हैं। इसमें आपका दिया हुआ इलाका होता है, आपका सही पता नहीं। पता केवल उस कोटेशन में बताया जाता है जिसे आप स्वीकार करें। | Messages | legal or safety |
| `quote-request-button` | Quote this request | इस अनुरोध पर कोटेशन दें | Messages | legal or safety |
| `messages-accept-button` | Accept these terms | ये शर्तें स्वीकार करें | Messages | legal or safety |
| `messages-decline-note` | This only changes what you see. The other side is not told, and no record is signed. Send them a message if you want them to know. | यह केवल उसे बदलता है जो आपको दिखता है। दूसरे पक्ष को नहीं बताया जाता, और कोई रिकॉर्ड हस्ताक्षरित नहीं होता। अगर आप चाहते हैं कि उन्हें पता चले, तो उन्हें संदेश भेजें। | Messages | legal or safety |
| `messages-decline-note-placeholder` | Optional note (for yourself only) | वैकल्पिक नोट (केवल आपके लिए) | Messages | legal or safety |
| `quote-address-disclosure-notice` | This address becomes part of a signed record that both parties keep and can export. It cannot be removed from a record already signed. | यह पता एक हस्ताक्षरित रिकॉर्ड का हिस्सा बन जाता है, जिसे दोनों पक्ष अपने पास रखते हैं और एक्सपोर्ट कर सकते हैं। एक बार हस्ताक्षर हो चुके रिकॉर्ड से इसे हटाया नहीं जा सकता। | Messages | legal or safety |
| `quote-default-cancellation` | 24 hours notice required for full refund | पूरी वापसी के लिए 24 घंटे पहले सूचना ज़रूरी है | Messages | legal or safety |
| `quote-default-refund` | Full refund if work not completed | काम पूरा न होने पर पूरी वापसी | Messages | legal or safety |
| `quote-default-dispute` | Small claims court or informal mediation | छोटे दावों की अदालत या आपसी सुलह | Messages | legal or safety |
| `quote-send-button` | Send quote | कोटेशन भेजें | Messages | legal or safety |
| `messages-delete-note-sent` | The local copy is removed and a deletion record kept. A request to delete it was sent to the other side; whether their client honours it is theirs to decide, and this cannot check. This installation's own message store still holds what it received. | स्थानीय प्रति हटा दी जाती है और हटाने का एक रिकॉर्ड रखा जाता है। दूसरे पक्ष को इसे हटाने का अनुरोध भेजा गया है; उनका क्लाइंट इसे मानेगा या नहीं, यह उनका फ़ैसला है, और इसकी जाँच यहाँ से नहीं हो सकती। इस इंस्टॉलेशन के अपने संदेश-भंडार में वह अब भी है जो उसे मिला था। | Messages | legal or safety |
| `messages-delete-note-received` | The local copy is removed and a deletion record kept. This is a message you received; the other side's copy is theirs. | स्थानीय प्रति हटा दी जाती है और हटाने का एक रिकॉर्ड रखा जाता है। यह वह संदेश है जो आपको मिला था; दूसरे पक्ष की प्रति उनकी अपनी है। | Messages | legal or safety |
| `membership-no-instant-removal` | A group's decision reaches copies other people already hold only when they next check. Nobody can promise it is removed everywhere at once. | किसी समूह का फ़ैसला उन प्रतियों तक, जो दूसरे लोगों के पास पहले से हैं, तभी पहुँचता है जब वे अगली बार जाँच करें। कोई यह वादा नहीं कर सकता कि वह हर जगह एक साथ हट जाएगा। | Membership words | legal or safety |
| `membership-withheld-revocation` | This shows every withdrawal the group has published. It cannot show one the group chose not to publish. | यह समूह की प्रकाशित की हुई हर वापसी दिखाता है। जो वापसी समूह ने प्रकाशित नहीं करने का फ़ैसला किया, वह यह नहीं दिखा सकता। | Membership words | legal or safety |

## All other rows (587)

| id | English | Hindi | screen | weight |
| --- | --- | --- | --- | --- |
| `language-picker-label` | Language | भाषा |  |  |
| `language-not-saved` | This browser does not let the Hub save the language, so it stays as it is. | यह ब्राउज़र हब को भाषा सहेजने नहीं देता, इसलिए भाषा वैसी ही रहेगी। |  |  |
| `age-moments` | moments ago | अभी-अभी |  |  |
| `money-input-invalid` | "{ $input }" is not an amount like { $example } for this currency | "{ $input }" इस मुद्रा के लिए सही राशि नहीं है, जैसे { $example } |  |  |
| `error-service-unavailable` | { $detail } | यह सेवा अभी उपलब्ध नहीं है। विवरण: { $detail } | Errors. `$detail` is the service'… |  |
| `error-service-timeout` | { $detail } | सेवा ने समय पर जवाब नहीं दिया। विवरण: { $detail } | Errors. `$detail` is the service'… |  |
| `error-permission` | { $detail } | आपको इसकी अनुमति नहीं है। विवरण: { $detail } | Errors. `$detail` is the service'… |  |
| `error-quota` | { $detail } | सीमा पूरी हो गई है। विवरण: { $detail } | Errors. `$detail` is the service'… |  |
| `error-not-signed-in` | { $detail } | आप साइन इन नहीं हैं। विवरण: { $detail } | Errors. `$detail` is the service'… |  |
| `error-not-owner` | { $detail } | यह काम केवल मालिक कर सकता है। विवरण: { $detail } | Errors. `$detail` is the service'… |  |
| `error-no-owner` | { $detail } | इस इंस्टॉलेशन का अभी कोई मालिक नहीं है। विवरण: { $detail } | Errors. `$detail` is the service'… |  |
| `error-not-local` | this installation refused a request that did not come from you | इस इंस्टॉलेशन ने ऐसा अनुरोध मना कर दिया जो आपकी ओर से नहीं आया था | Errors. `$detail` is the service'… |  |
| `error-method-not-found` | { $detail } | यह अनुरोध इस इंस्टॉलेशन में नहीं मिला। विवरण: { $detail } | Errors. `$detail` is the service'… |  |
| `error-invalid-input` | { $detail } | दी गई जानकारी सही नहीं है। विवरण: { $detail } | Errors. `$detail` is the service'… |  |
| `error-internal` | { $detail } | सेवा में गड़बड़ी हुई। विवरण: { $detail } | Errors. `$detail` is the service'… |  |
| `error-service` | { $detail } | अनुरोध पूरा नहीं हो सका। विवरण: { $detail } | Errors. `$detail` is the service'… |  |
| `error-network` | Could not reach this installation: { $detail } | इस इंस्टॉलेशन तक नहीं पहुँच सके। विवरण: { $detail } | Errors. `$detail` is the service'… |  |
| `error-http` | HTTP error { $code } | सर्वर से त्रुटि मिली (HTTP { $code }) | Errors. `$detail` is the service'… |  |
| `login-failed` | Sign-in failed: { $detail } | साइन इन नहीं हो सका: { $detail } | Failures shown beside an action. … |  |
| `directory-search-failed` | The search failed: { $detail }. Press { $button } to try again. | खोज नहीं हो सकी: { $detail }। फिर कोशिश करने के लिए "{ $button }" दबाएँ। | Failures shown beside an action. … |  |
| `directory-search-not-started` | This installation could not start the search. Press { $button } to try again. | यह इंस्टॉलेशन खोज शुरू नहीं कर सका। फिर कोशिश करने के लिए "{ $button }" दबाएँ। | Failures shown beside an action. … |  |
| `contacts-upsert-failed` | Upsert failed: { $detail } | संपर्क सहेजा नहीं जा सका: { $detail } | Failures shown beside an action. … |  |
| `safety-block-failed` | Block failed: { $detail } | ब्लॉक नहीं हो सका: { $detail } | Failures shown beside an action. … |  |
| `safety-report-failed` | Report failed: { $detail } | रिपोर्ट नहीं भेजी जा सकी: { $detail } | Failures shown beside an action. … |  |
| `safety-save-failed` | Save failed: { $detail } | सहेजा नहीं जा सका: { $detail } | Failures shown beside an action. … |  |
| `backup-export-failed` | Export failed: { $detail } | एक्सपोर्ट नहीं हो सका: { $detail } | Failures shown beside an action. … |  |
| `backup-import-failed` | Import failed: { $detail } | इम्पोर्ट नहीं हो सका: { $detail } | Failures shown beside an action. … |  |
| `profile-error` | Error: { $detail } | त्रुटि: { $detail } | Failures shown beside an action. … |  |
| `payment-claimed` | The payer says they paid. | भुगतान करने वाले का कहना है कि उसने भुगतान कर दिया। | Sentences that the Rust side also… |  |
| `payment-acknowledged` | The payee confirms they received the payment. | पैसे पाने वाला पुष्टि करता है कि उसे भुगतान मिल गया। | Sentences that the Rust side also… |  |
| `fulfilment-claimed` | The person doing the work says it is done. | काम करने वाले का कहना है कि काम पूरा हो गया। | Sentences that the Rust side also… |  |
| `fulfilment-acknowledged` | The person receiving the work confirms it is done. | काम पाने वाला पुष्टि करता है कि काम पूरा हो गया। | Sentences that the Rust side also… |  |
| `track-unconfirmed` | No confirmation was recorded before the window closed. | समय-सीमा खत्म होने से पहले कोई पुष्टि दर्ज नहीं हुई। | Sentences that the Rust side also… |  |
| `group-add-unreachable` | Could not reach this person to add them. Someone you have not talked to before must be online when you add them. | इस व्यक्ति तक पहुँच नहीं सके, इसलिए उन्हें जोड़ा नहीं जा सका। जिससे आपने पहले बात नहीं की, उसे जोड़ते समय उसका ऑनलाइन होना ज़रूरी है। |  |  |
| `group-transcript-check-notice` | Members who see the same code hold the same messages in the same order. | जिन सदस्यों को एक ही कोड दिखता है, उनके पास वही संदेश उसी क्रम में हैं। |  |  |
| `group-cards-not-allowed` | Cards are sent only in a 1:1 conversation. | कार्ड केवल आमने-सामने की (1:1) बातचीत में भेजे जाते हैं। |  |  |
| `group-session-ended-notice` | Your session ended, for example because this installation restarted. Log in again. | आपका सत्र खत्म हो गया, जैसे कि इंस्टॉलेशन दोबारा चालू होने से। फिर से लॉग इन करें। | Group lines |  |
| `group-event-created` | { $author } created the group | { $author } ने समूह बनाया | Group lines |  |
| `group-event-added` | { $author } added { $subject } | { $author } ने { $subject } को जोड़ा | Group lines |  |
| `group-event-removed` | { $author } removed { $subject } | { $author } ने { $subject } को हटाया | Group lines |  |
| `group-event-updated` | { $author } updated the group | { $author } ने समूह में बदलाव किया | Group lines |  |
| `card-unknown-title` | Unknown Card ({ $type } v{ $version }) | अज्ञात कार्ड ({ $type } v{ $version }) | Cards. A label that is followed b… |  |
| `card-unknown-body` | This client does not understand this card type or version. | यह क्लाइंट इस कार्ड के प्रकार या संस्करण को नहीं समझता। | Cards. A label that is followed b… |  |
| `card-refused-title` | Unverified { $type } card (v{ $version }) | असत्यापित { $type } कार्ड (v{ $version }) | Cards. A label that is followed b… |  |
| `card-refused-notice` | This node received a card of this type, but could not verify it. It is shown so you know it was sent, but cannot be accepted or acted upon. | इस नोड को इस प्रकार का एक कार्ड मिला, पर वह उसे सत्यापित नहीं कर सका। यह इसलिए दिखाया गया है कि आपको पता चले कि वह भेजा गया था, पर उसे स्वीकार नहीं किया जा सकता और उस पर कोई कार्रवाई नहीं की जा सकती। | Cards. A label that is followed b… |  |
| `card-refused-reason` | Reason: { $reason } | कारण: { $reason } | Cards. A label that is followed b… |  |
| `request-title` | Request | अनुरोध |  |  |
| `request-categories` | Categories: { $categories } | श्रेणियाँ: { $categories } |  |  |
| `request-listing` | Listing: { $id } | लिस्टिंग: { $id } |  |  |
| `request-window-label` | Window: | समय-सीमा: |  |  |
| `request-notice` | Notice: { $value } | सूचना: { $value } |  |  |
| `progress-title` | Booking Progress | बुकिंग की स्थिति |  |  |
| `progress-status` | Status: { $state } | स्थिति: { $state } |  |  |
| `progress-state-scheduled` | Scheduled | तय हुआ |  |  |
| `progress-state-in-progress` | In progress | जारी है |  |  |
| `progress-state-completed` | Completed | पूरा हुआ |  |  |
| `progress-state-cancelled` | Cancelled | रद्द |  |  |
| `progress-state-conflict` | Conflict | टकराव |  |  |
| `progress-state-ended-unconfirmed` | Ended, unconfirmed | खत्म, पुष्टि नहीं हुई |  |  |
| `progress-conflict-slot-taken` | Another booking took this slot first. | इस समय पर पहले किसी और की बुकिंग हो गई। |  |  |
| `progress-conflict-slot-unavailable` | The listing owner removed this slot. | लिस्टिंग के मालिक ने यह समय हटा दिया। |  |  |
| `progress-cancel-reason` | Cancellation reason: { $reason } | रद्द करने का कारण: { $reason } |  |  |
| `payment-request-title` | Payment Request | भुगतान का अनुरोध |  |  |
| `payment-ack-title` | Payment Acknowledgement | भुगतान की पावती |  |  |
| `payment-amount` | Amount: { $amount } | राशि: { $amount } |  |  |
| `payment-note` | Note: { $note } | नोट: { $note } |  |  |
| `payment-observed-label` | Observed: | देखा गया: |  |  |
| `payment-method` | Method: { $method } | तरीका: { $method } |  |  |
| `payment-reference` | Reference: { $reference } | संदर्भ: { $reference } |  |  |
| `payment-corrected` | This is a corrected statement. | यह एक सुधारा हुआ बयान है। |  |  |
| `fulfilment-title` | Fulfilment Receipt | काम पूरा होने की रसीद |  |  |
| `place-editor-at-giver` | At my place | मेरी जगह पर | Places and audiences of a listing |  |
| `place-editor-at-receiver` | At their place | उनकी जगह पर | Places and audiences of a listing |  |
| `place-remote` | Remote | ऑनलाइन (दूर से) | Places and audiences of a listing |  |
| `place-card-at-giver` | At the quote author's place | कोटेशन लिखने वाले की जगह पर | Places and audiences of a listing |  |
| `place-card-at-receiver` | At the counterparty's place | दूसरे पक्ष की जगह पर | Places and audiences of a listing |  |
| `audience-anyone` | Anyone | कोई भी | Places and audiences of a listing |  |
| `audience-members` | Members | सदस्य | Places and audiences of a listing |  |
| `audience-referral` | Referral | सिफ़ारिश से | Places and audiences of a listing |  |
| `audience-past-counterparties` | People I have worked with before | जिनके साथ मैंने पहले काम किया है | Places and audiences of a listing |  |
| `hub-title` | Roym Hub | Roym हब | The Hub shell: title, session bar… |  |
| `session-logged-in` | Logged in: { $did } ({ $auth }) | लॉग इन: { $did } ({ $auth }) | The Hub shell: title, session bar… |  |
| `session-logout` | Log out | लॉग आउट | The Hub shell: title, session bar… |  |
| `session-anonymous` | Not logged in | लॉग इन नहीं है | The Hub shell: title, session bar… |  |
| `tab-error` | Could not load this tab: { $detail } | यह टैब लोड नहीं हो सका: { $detail } | The Hub shell: title, session bar… |  |
| `tab-profile` | Profile | प्रोफ़ाइल | The Hub shell: title, session bar… |  |
| `tab-contacts` | Contacts | संपर्क | The Hub shell: title, session bar… |  |
| `tab-messages` | Messages | संदेश | The Hub shell: title, session bar… |  |
| `tab-groups` | Groups | समूह | The Hub shell: title, session bar… |  |
| `tab-listings` | Listings | लिस्टिंग | The Hub shell: title, session bar… |  |
| `tab-directory` | Directory | डायरेक्टरी | The Hub shell: title, session bar… |  |
| `tab-memberships` | Memberships | सदस्यता | The Hub shell: title, session bar… |  |
| `tab-synorg` | SynOrg | SynOrg | The Hub shell: title, session bar… |  |
| `tab-safety` | Safety | सुरक्षा | The Hub shell: title, session bar… |  |
| `tab-backup` | Backup | बैकअप | The Hub shell: title, session bar… |  |
| `tab-components` | Components | कंपोनेंट | The Hub shell: title, session bar… |  |
| `login-title` | Sign in | साइन इन | The Hub shell: title, session bar… |  |
| `login-auth-unreachable` | Could not reach the auth service on this node. | इस नोड की साइन इन सेवा तक नहीं पहुँच सके। | The Hub shell: title, session bar… |  |
| `login-no-method` | This node has no browser login method enabled. | इस नोड पर ब्राउज़र से साइन इन करने का कोई तरीका चालू नहीं है। | The Hub shell: title, session bar… |  |
| `login-intro` | Load a session key file produced by `roymctl session delegate --as <name> --registry-url <registry>`. The key stays in this browser and cannot be read back by page code. | `roymctl session delegate --as <name> --registry-url <registry>` से बनी सत्र-कुंजी की फ़ाइल लोड करें। कुंजी इसी ब्राउज़र में रहती है और पेज का कोड उसे वापस नहीं पढ़ सकता। | The Hub shell: title, session bar… |  |
| `login-with-stored` | Sign in with this browser's key | इस ब्राउज़र की कुंजी से साइन इन करें | The Hub shell: title, session bar… |  |
| `login-signing-in` | Signing in... | साइन इन हो रहा है... | The Hub shell: title, session bar… |  |
| `login-stored-gone` | The stored key is gone; load a session key file. | सहेजी हुई कुंजी नहीं मिली; सत्र-कुंजी की फ़ाइल लोड करें। | The Hub shell: title, session bar… |  |
| `login-load-file` | Load session key file | सत्र-कुंजी की फ़ाइल लोड करें | The Hub shell: title, session bar… |  |
| `gallery-home-title` | Roym Hub Home | Roym हब होम | The Hub shell: title, session bar… |  |
| `gallery-did` | Authenticated DID: { $did } | प्रमाणित DID: { $did } | The Hub shell: title, session bar… |  |
| `gallery-contacting` | Contacting profile service... | प्रोफ़ाइल सेवा से संपर्क हो रहा है... | The Hub shell: title, session bar… |  |
| `gallery-ping-response` | Profile ping response: { $response } | प्रोफ़ाइल पिंग का जवाब: { $response } | The Hub shell: title, session bar… |  |
| `gallery-ping-failed` | Profile ping failed: HTTP { $status } | प्रोफ़ाइल पिंग विफल: HTTP { $status } | The Hub shell: title, session bar… |  |
| `gallery-ping-error` | Profile ping error: { $error } | प्रोफ़ाइल पिंग में त्रुटि: { $error } | The Hub shell: title, session bar… |  |
| `gallery-heading` | Card Gallery (Sample Cards) | कार्ड गैलरी (नमूना कार्ड) | The Hub shell: title, session bar… |  |
| `date-invalid` | Invalid Date | अमान्य तारीख | Shown where a time from another n… |  |
| `directory-signed-age` | Signed { $age } | हस्ताक्षर: { $age } | Lines of the Directory and SynOrg… |  |
| `synorg-credential-expires` | expires { $date } | वैधता खत्म: { $date } | Lines of the Directory and SynOrg… |  |
| `synorg-until` | until { $date } | { $date } तक | Lines of the Directory and SynOrg… |  |
| `synorg-until-lifted` | until lifted | जब तक हटाया न जाए | Lines of the Directory and SynOrg… |  |
| `error-unexpected` | { $detail } | कुछ गड़बड़ हो गई। विवरण: { $detail } | Shared |  |
| `error-no-detail` | RPC error | सेवा से त्रुटि मिली | Shared |  |
| `login-error-odd-hex` | odd-length hex string | हेक्स पाठ में अंकों की संख्या सम नहीं है | Sign-in |  |
| `login-error-key-length` | temporary secret key must be 32 bytes | अस्थायी गुप्त कुंजी 32 बाइट की होनी चाहिए | Sign-in |  |
| `login-error-challenge` | challenge failed: HTTP { $status } | जाँच का अनुरोध नहीं हो सका: HTTP { $status } | Sign-in |  |
| `login-error-no-assertion` | auth service did not return an assertion to sign | प्रमाणीकरण सेवा ने हस्ताक्षर के लिए कुछ नहीं भेजा | Sign-in |  |
| `login-error-http` | HTTP { $status } | HTTP { $status } | Sign-in |  |
| `login-error-service` | { $detail } | सेवा ने मना कर दिया। विवरण: { $detail } | Sign-in |  |
| `login-error-methods` | methods failed: HTTP { $status } | तरीकों की सूची नहीं मिली: HTTP { $status } | Sign-in |  |
| `common-saved` | Saved. | सहेज लिया गया। | Sign-in |  |
| `common-save-failed` | Save failed: { $detail } | सहेजा नहीं जा सका: { $detail } | Sign-in |  |
| `common-remove` | Remove | हटाएँ | Sign-in |  |
| `profile-title` | Profile | प्रोफ़ाइल | Profile |  |
| `profile-display-name` | Display Name: | दिखने वाला नाम: | Profile |  |
| `profile-conversation-address` | Conversation Address: | बातचीत का पता: | Profile |  |
| `profile-save` | Save Profile | प्रोफ़ाइल सहेजें | Profile |  |
| `profile-saving` | Saving... | सहेजा जा रहा है... | Profile |  |
| `profile-saved` | Saved record { $record } (issuer: { $issuer }) | रिकॉर्ड { $record } सहेजा गया (जारी करने वाला: { $issuer }) | Profile |  |
| `contacts-title` | Contacts | संपर्क | Contacts |  |
| `contacts-load-failed` | Could not load contacts. | संपर्क लोड नहीं हो सके। | Contacts |  |
| `contacts-add-heading` | Add / Import Contact | संपर्क जोड़ें / इम्पोर्ट करें | Contacts |  |
| `contacts-did-placeholder` | Person DID | व्यक्ति का DID | Contacts |  |
| `contacts-address-placeholder` | Conversation Address (required if no profile envelope) | बातचीत का पता (प्रोफ़ाइल लिफ़ाफ़ा न हो तो ज़रूरी) | Contacts |  |
| `contacts-envelope-placeholder` | Paste Profile Envelope JSON (optional) | प्रोफ़ाइल लिफ़ाफ़े का JSON यहाँ चिपकाएँ (ज़रूरी नहीं) | Contacts |  |
| `contacts-upsert-button` | Upsert Contact | संपर्क जोड़ें या बदलें | Contacts |  |
| `contacts-envelope-invalid` | Profile envelope is not valid JSON | प्रोफ़ाइल लिफ़ाफ़ा सही JSON नहीं है | Contacts |  |
| `setup-title` | Signing Certificate Required | हस्ताक्षर प्रमाणपत्र ज़रूरी है | Set-up |  |
| `setup-intro` | This installation has no active record-signing certificate enrolled. To enrol, run `roymctl roym enrol-signing --master <identity> --registry-url <registry>` from your terminal. The browser cannot do this because it holds no master key. | इस इंस्टॉलेशन में रिकॉर्ड पर हस्ताक्षर करने का कोई चालू प्रमाणपत्र दर्ज नहीं है। दर्ज करने के लिए अपने टर्मिनल से `roymctl roym enrol-signing --master <identity> --registry-url <registry>` चलाएँ। ब्राउज़र यह नहीं कर सकता, क्योंकि उसके पास मुख्य कुंजी नहीं होती। | Set-up |  |
| `setup-missing` | Missing signing certificates for: { $services } | इनके हस्ताक्षर प्रमाणपत्र नहीं मिले: { $services } | Set-up |  |
| `setup-check-again` | Check again | फिर जाँचें | Set-up |  |
| `backup-title` | App Data Backup & Restore | ऐप डेटा का बैकअप और बहाली | Backup |  |
| `backup-intro` | This exports and imports application data. It never exports or touches your identity secret key. | यह ऐप का डेटा एक्सपोर्ट और इम्पोर्ट करता है। यह आपकी पहचान की गुप्त कुंजी को कभी एक्सपोर्ट नहीं करता और न छूता है। | Backup |  |
| `backup-separate-note` | Each bundle below is signed by you when it is exported, and is refused on import if it was changed. An encrypted backup of everything, including your identity, is made with `roymctl roym backup create`. | नीचे हर बंडल एक्सपोर्ट के समय आपके हस्ताक्षर से बंद होता है, और बदला गया हो तो इम्पोर्ट पर मना कर दिया जाता है। आपकी पहचान समेत सब कुछ का एन्क्रिप्टेड बैकअप `roymctl roym backup create` से बनता है। | Backup |  |
| `backup-bundle-profile` | Profile, contacts, and safety records | प्रोफ़ाइल, संपर्क और सुरक्षा के रिकॉर्ड | Backup |  |
| `backup-bundle-conversation` | Conversations and messages | बातचीत और संदेश | Backup |  |
| `backup-bundle-catalog` | Listings and availability | लिस्टिंग और उपलब्ध समय | Backup |  |
| `backup-bundle-transaction` | Requests, quotes, and agreements | अनुरोध, कोटेशन और समझौते | Backup |  |
| `backup-bundle-directory` | This installation's SynOrg directory, if it runs one | इस इंस्टॉलेशन की SynOrg डायरेक्टरी, अगर वह चलाता हो | Backup |  |
| `backup-export` | Export | एक्सपोर्ट | Backup |  |
| `backup-import` | Import | इम्पोर्ट | Backup |  |
| `backup-reading` | Reading file... | फ़ाइल पढ़ी जा रही है... | Backup |  |
| `backup-importing` | Importing... | इम्पोर्ट हो रहा है... | Backup |  |
| `backup-imported` | Imported: { $count } record(s). | इम्पोर्ट हुए: { $count } रिकॉर्ड। | Backup |  |
| `backup-imported-unknown` | Imported: bundle record(s). | बंडल के रिकॉर्ड इम्पोर्ट हुए। | Backup |  |
| `safety-title` | Safety & Blocklist | सुरक्षा और ब्लॉक सूची | Safety |  |
| `safety-policy-unavailable` | Could not load this node's safety policy. | इस नोड की सुरक्षा नीति लोड नहीं हो सकी। | Safety |  |
| `safety-block-placeholder` | Person DID or Address to block | ब्लॉक करने के लिए व्यक्ति का DID या पता | Safety |  |
| `safety-block-button` | Block | ब्लॉक करें | Safety |  |
| `safety-blocked-line` | Blocked: { $who } | ब्लॉक किया गया: { $who } | Safety |  |
| `safety-block-list-failed` | Could not load block list. | ब्लॉक सूची लोड नहीं हो सकी। | Safety |  |
| `safety-report-heading` | Report a person, listing, or message | किसी व्यक्ति, लिस्टिंग या संदेश की रिपोर्ट करें | Safety |  |
| `safety-kind-person` | person | व्यक्ति | Safety |  |
| `safety-kind-listing` | listing | लिस्टिंग | Safety |  |
| `safety-kind-message` | message | संदेश | Safety |  |
| `safety-subject-placeholder` | Subject DID or id | जिसकी रिपोर्ट है उसका DID या आईडी | Safety |  |
| `safety-category-impersonation` | impersonation | किसी और बनकर पेश आना | Safety |  |
| `safety-category-fraud` | fraud | धोखाधड़ी | Safety |  |
| `safety-category-harassment` | harassment | परेशान करना | Safety |  |
| `safety-category-unsafe-service` | unsafe-service | असुरक्षित सेवा | Safety |  |
| `safety-category-illegal-content` | illegal-content | गैरकानूनी सामग्री | Safety |  |
| `safety-details-placeholder` | What happened (optional) | क्या हुआ (ज़रूरी नहीं) | Safety |  |
| `safety-file-report` | File report | रिपोर्ट दर्ज करें | Safety |  |
| `safety-report-row` | { $category } - { $kind } { $subject } [{ $status }] | { $category } - { $kind } { $subject } [{ $status }] | Safety |  |
| `safety-status-open` | open | खुली | Safety |  |
| `safety-status-withdrawn` | withdrawn | वापस ली गई | Safety |  |
| `safety-withdraw` | Withdraw | वापस लें | Safety |  |
| `safety-reports-load-failed` | Could not load reports. | रिपोर्टें लोड नहीं हो सकीं। | Safety |  |
| `safety-subject-required` | A subject is required. | जिसकी रिपोर्ट है उसे बताना ज़रूरी है। | Safety |  |
| `safety-recorded` | Recorded { $id } ({ $status }) | दर्ज हुई { $id } ({ $status }) | Safety |  |
| `safety-limit-heading` | First-contact rate limit | पहली बार संपर्क की सीमा | Safety |  |
| `safety-limit-intro` | How many strangers may start a conversation with you inside one window. An ongoing conversation is never limited. | एक समय-सीमा के अंदर कितने अनजान लोग आपसे बातचीत शुरू कर सकते हैं। चल रही बातचीत पर कभी कोई सीमा नहीं लगती। | Safety |  |
| `safety-window-placeholder` | window (seconds) | समय-सीमा (सेकंड में) | Safety |  |
| `safety-max-placeholder` | max new contacts per window | हर समय-सीमा में नए संपर्कों की अधिकतम संख्या | Safety |  |
| `safety-save-limit` | Save limit | सीमा सहेजें | Safety |  |
| `safety-limit-load-failed` | Could not load the current limit. | मौजूदा सीमा लोड नहीं हो सकी। | Safety |  |
| `safety-whole-numbers` | Both values must be whole numbers. | दोनों मान पूर्ण संख्याएँ होने चाहिए। | Safety |  |
| `listing-screen-title` | Listings | लिस्टिंग | Listings |  |
| `listing-your-heading` | Your listings | आपकी लिस्टिंग | Listings |  |
| `listing-load-failed` | Could not load listings: { $detail } | लिस्टिंग लोड नहीं हो सकीं: { $detail } | Listings |  |
| `listing-empty` | No listings yet. | अभी कोई लिस्टिंग नहीं है। | Listings |  |
| `listing-version` | v{ $count } | v{ $count } | Listings |  |
| `listing-history-button` | History | इतिहास | Listings |  |
| `listing-history-line` | #{ $n } { $id } (supersedes { $previous }) | #{ $n } { $id } (इसने { $previous } की जगह ली) | Listings |  |
| `listing-history-first` | #{ $n } { $id } (first version) | #{ $n } { $id } (पहला संस्करण) | Listings |  |
| `listing-history-failed` | History failed: { $detail } | इतिहास लोड नहीं हो सका: { $detail } | Listings |  |
| `listing-withdraw` | Withdraw | वापस लें | Listings |  |
| `listing-withdraw-failed` | Withdraw failed: { $detail } | वापस नहीं लिया जा सका: { $detail } | Listings |  |
| `listing-publish-heading` | Publish to a directory | डायरेक्टरी में प्रकाशित करें | Listings |  |
| `listing-publish-button` | Publish here | यहाँ प्रकाशित करें | Listings |  |
| `listing-published` | Published. | प्रकाशित हो गया। | Listings |  |
| `listing-not-published` | Not published: { $reason } | प्रकाशित नहीं हुआ: { $reason } | Listings |  |
| `listing-editor-heading` | New / edit a listing | नई लिस्टिंग बनाएँ / बदलें | Listings |  |
| `listing-save-button` | Save listing | लिस्टिंग सहेजें | Listings |  |
| `listing-saved` | Saved { $id } as version { $version } ({ $record }) | { $id } सहेजी गई, संस्करण { $version } ({ $record }) | Listings |  |
| `listing-availability-heading` | Availability (for slot bookings) | उपलब्ध समय (स्लॉट बुकिंग के लिए) | Listings |  |
| `listing-add-slot` | Add slot | स्लॉट जोड़ें | Listings |  |
| `listing-slot-line` | { $start } - { $end } (x{ $capacity }) | { $start } - { $end } (x{ $capacity }) | Listings |  |
| `listing-slots-load-failed` | Could not load slots: { $detail } | स्लॉट लोड नहीं हो सके: { $detail } | Listings |  |
| `listing-verify-heading` | Check a listing someone sent me | किसी की भेजी हुई लिस्टिंग जाँचें | Listings |  |
| `listing-verify-placeholder` | Paste a listing envelope JSON | लिस्टिंग लिफ़ाफ़े का JSON यहाँ चिपकाएँ | Listings |  |
| `listing-verify-button` | Check | जाँचें | Listings |  |
| `listing-verify-bad-json` | That is not valid JSON. | यह सही JSON नहीं है। | Listings |  |
| `listing-verified` | Verified. | सत्यापित। | Listings |  |
| `listing-verify-issuer` | Issuer: { $issuer } | जारी करने वाला: { $issuer } | Listings |  |
| `listing-verify-address` | Conversation address: { $address } | बातचीत का पता: { $address } | Listings |  |
| `listing-verify-status` | Status: { $status } | स्थिति: { $status } | Listings |  |
| `listing-not-verified` | Not verified -- treat as unknown. | सत्यापित नहीं -- अनजान मानें। | Listings |  |
| `listing-check-failed` | Check failed: { $detail } | जाँच नहीं हो सकी: { $detail } | Listings |  |
| `listing-label-title` | Title | शीर्षक | Listings |  |
| `listing-label-slug` | Slug | स्लग | Listings |  |
| `listing-label-summary` | Summary | सार | Listings |  |
| `listing-label-categories` | Categories | श्रेणियाँ | Listings |  |
| `listing-label-address` | Conversation address | बातचीत का पता | Listings |  |
| `listing-label-status` | Status | स्थिति | Listings |  |
| `listing-label-mode` | Mode | तरीका | Listings |  |
| `listing-label-lead-time` | Lead time (s) | पहले से सूचना (सेकंड) | Listings |  |
| `listing-label-cancellation-window` | Cancellation window (s) | रद्द करने की अवधि (सेकंड) | Listings |  |
| `listing-label-max-per-booking` | Max per booking | हर बुकिंग में अधिकतम | Listings |  |
| `listing-label-currency` | Currency | मुद्रा | Listings |  |
| `listing-label-model` | Model | मूल्य का तरीका | Listings |  |
| `listing-label-amount` | Amount | राशि | Listings |  |
| `listing-label-fees` | Fees | शुल्क | Listings |  |
| `listing-label-methods` | Methods | भुगतान के तरीके | Listings |  |
| `listing-label-payee` | Payee | भुगतान पाने वाला | Listings |  |
| `listing-label-tax-included` | tax included | कर शामिल | Listings |  |
| `listing-label-unit` | Unit | इकाई | Listings |  |
| `listing-label-pack-size` | Pack size | पैक का आकार | Listings |  |
| `listing-label-condition` | Condition | हालत | Listings |  |
| `listing-label-sku` | SKU | स्टॉक कोड | Listings |  |
| `listing-label-duration` | Duration (s) | अवधि (सेकंड) | Listings |  |
| `listing-label-includes` | Includes | शामिल | Listings |  |
| `listing-label-excludes` | Excludes | शामिल नहीं | Listings |  |
| `listing-label-prerequisites` | Prerequisites | पहले से ज़रूरी बातें | Listings |  |
| `listing-label-where` | Where | कहाँ | Listings |  |
| `listing-label-disclosure` | Address disclosure | पता कब बताया जाए | Listings |  |
| `listing-label-service-area` | Service area | सेवा का क्षेत्र | Listings |  |
| `listing-label-open-to` | Open to | किसके लिए खुला | Listings |  |
| `listing-label-group-did` | Group DID | समूह का DID | Listings |  |
| `listing-label-fulfilment-receipt` | issues a fulfilment receipt | काम पूरा होने की रसीद देता है | Listings |  |
| `listing-label-warranty` | Warranty (s) | वारंटी (सेकंड) | Listings |  |
| `listing-label-retention` | Retention (s) | रखने की अवधि (सेकंड) | Listings |  |
| `listing-label-start` | Start | शुरू | Listings |  |
| `listing-label-end` | End | खत्म | Listings |  |
| `listing-label-capacity` | Capacity | क्षमता | Listings |  |
| `listing-placeholder-slug` | slug (optional; derived from the title) | स्लग (ज़रूरी नहीं; शीर्षक से बन जाता है) | Listings |  |
| `listing-placeholder-title` | title | शीर्षक | Listings |  |
| `listing-placeholder-summary` | summary | सार | Listings |  |
| `listing-placeholder-categories` | categories, comma separated | श्रेणियाँ, कॉमा से अलग करें | Listings |  |
| `listing-placeholder-address` | conversation address (optional; taken from your profile) | बातचीत का पता (ज़रूरी नहीं; प्रोफ़ाइल से लिया जाता है) | Listings |  |
| `listing-placeholder-lead-time` | lead time (seconds) | पहले से सूचना (सेकंड) | Listings |  |
| `listing-placeholder-cancellation-window` | cancellation window (seconds) | रद्द करने की अवधि (सेकंड) | Listings |  |
| `listing-placeholder-max-per-booking` | max per booking | हर बुकिंग में अधिकतम | Listings |  |
| `listing-placeholder-currency` | currency (ISO-4217, e.g. EUR) | मुद्रा का कोड (ISO-4217, जैसे EUR) | Listings |  |
| `listing-placeholder-amount` | amount, e.g. 35.50 | राशि, जैसे 35.50 | Listings |  |
| `listing-placeholder-fees` | fees, e.g. 2.50 (optional) | शुल्क, जैसे 2.50 (ज़रूरी नहीं) | Listings |  |
| `listing-placeholder-methods` | methods, comma separated | भुगतान के तरीके, कॉमा से अलग करें | Listings |  |
| `listing-placeholder-payee` | payee (informational only) | भुगतान पाने वाला (केवल जानकारी के लिए) | Listings |  |
| `listing-placeholder-unit` | unit, e.g. item | इकाई, जैसे नग | Listings |  |
| `listing-placeholder-pack-size` | pack size | पैक का आकार | Listings |  |
| `listing-placeholder-sku` | SKU (optional) | स्टॉक कोड (ज़रूरी नहीं) | Listings |  |
| `listing-placeholder-duration` | duration (seconds) | अवधि (सेकंड) | Listings |  |
| `listing-placeholder-includes` | includes, comma separated | शामिल बातें, कॉमा से अलग करें | Listings |  |
| `listing-placeholder-excludes` | excludes, comma separated | जो शामिल नहीं है, कॉमा से अलग करें | Listings |  |
| `listing-placeholder-prerequisites` | prerequisites, comma separated | पहले से ज़रूरी बातें, कॉमा से अलग करें | Listings |  |
| `listing-placeholder-area` | service area label (optional) | सेवा के क्षेत्र का नाम (ज़रूरी नहीं) | Listings |  |
| `listing-placeholder-group-did` | group DID (required for members) | समूह का DID (सदस्यों के लिए ज़रूरी) | Listings |  |
| `listing-placeholder-warranty` | warranty (seconds) | वारंटी (सेकंड) | Listings |  |
| `listing-placeholder-retention` | retention (seconds) | रखने की अवधि (सेकंड) | Listings |  |
| `listing-placeholder-capacity` | capacity | क्षमता | Listings |  |
| `listing-placeholder-directory-did` | Roym Directory service DID | Roym डायरेक्टरी सेवा का DID | Listings |  |
| `listing-status-active` | active | सक्रिय | Listings |  |
| `listing-status-draft` | draft | मसौदा | Listings |  |
| `listing-status-withdrawn` | withdrawn | वापस ली गई | Listings |  |
| `listing-mode-enquiry` | enquiry | पूछताछ | Listings |  |
| `listing-mode-slots` | slots | स्लॉट | Listings |  |
| `listing-mode-order` | order | ऑर्डर | Listings |  |
| `listing-model-per-hour` | per-hour | प्रति घंटा | Listings |  |
| `listing-model-fixed` | fixed | तय कीमत | Listings |  |
| `listing-model-per-unit` | per-unit | प्रति इकाई | Listings |  |
| `listing-model-quote-only` | quote-only | केवल कोटेशन | Listings |  |
| `listing-condition-new` | new | नया | Listings |  |
| `listing-condition-used` | used | इस्तेमाल किया हुआ | Listings |  |
| `listing-condition-refurbished` | refurbished | दोबारा तैयार किया हुआ | Listings |  |
| `listing-disclosure-on-agreement` | on-agreement | समझौते के बाद | Listings |  |
| `listing-disclosure-public` | public | सबको दिखे | Listings |  |
| `listing-block-booking` | booking | बुकिंग | Listings |  |
| `listing-block-payment` | payment | भुगतान | Listings |  |
| `listing-block-product` | product | उत्पाद | Listings |  |
| `listing-block-service` | service | सेवा | Listings |  |
| `listing-block-location` | location | जगह | Listings |  |
| `listing-block-relationship` | relationship | संबंध | Listings |  |
| `listing-block-service-record` | service_record | सेवा का रिकॉर्ड | Listings |  |
| `listing-field-lead-time` | lead time | पहले से सूचना का समय | Listings |  |
| `listing-field-cancellation-window` | cancellation window | रद्द करने की अवधि | Listings |  |
| `listing-field-max-per-booking` | max per booking | हर बुकिंग में अधिकतम संख्या | Listings |  |
| `listing-field-pack-size` | pack size | पैक का आकार | Listings |  |
| `listing-field-duration` | duration | अवधि | Listings |  |
| `listing-field-warranty` | warranty | वारंटी | Listings |  |
| `listing-field-retention` | retention | रखने की अवधि | Listings |  |
| `listing-field-capacity` | capacity | क्षमता | Listings |  |
| `listing-error-coordinate` | "{ $input }" is not a coordinate in degrees | "{ $input }" अंशों में सही निर्देशांक नहीं है | Listings |  |
| `listing-error-whole` | { $field } must be a whole number of seconds or units | { $field } सेकंड या इकाइयों की पूर्ण संख्या होनी चाहिए | Listings |  |
| `listing-error-required` | { $field } is required | { $field } ज़रूरी है | Listings |  |
| `listing-error-currency` | unknown currency '{ $currency }' | अज्ञात मुद्रा '{ $currency }' | Listings |  |
| `listing-error-amount-required` | an amount is required unless the model is quote-only | जब तक मूल्य का तरीका केवल-कोटेशन न हो, राशि ज़रूरी है | Listings |  |
| `listing-error-quote-only-amount` | a quote-only listing must not carry an amount | केवल-कोटेशन वाली लिस्टिंग में राशि नहीं होनी चाहिए | Listings |  |
| `listing-error-members-group` | a members-only listing needs the group DID | केवल-सदस्यों वाली लिस्टिंग के लिए समूह का DID चाहिए | Listings |  |
| `listing-error-title-required` | a title is required | शीर्षक ज़रूरी है | Listings |  |
| `listing-error-slot-times` | a slot needs a start and an end time | स्लॉट के लिए शुरू और खत्म होने का समय चाहिए | Listings |  |
| `listing-error-slot-order` | a slot's end must be after its start | स्लॉट का खत्म होने का समय शुरू होने के बाद का होना चाहिए | Listings |  |
| `listing-error-slot-capacity` | a slot needs a capacity of at least 1 | स्लॉट की क्षमता कम से कम 1 होनी चाहिए | Listings |  |
| `messages-title` | Messages | संदेश | Messages |  |
| `messages-open-placeholder` | A conversation address (from a listing or a contact) | बातचीत का पता (किसी लिस्टिंग या संपर्क से) | Messages |  |
| `messages-open-button` | Open conversation | बातचीत खोलें | Messages |  |
| `messages-open-failed` | Could not open: { $detail } | खोला नहीं जा सका: { $detail } | Messages |  |
| `messages-search-placeholder` | Search your messages | अपने संदेश खोजें | Messages |  |
| `messages-search-button` | Search | खोजें | Messages |  |
| `messages-search-empty` | No messages found. | कोई संदेश नहीं मिला। | Messages |  |
| `messages-search-failed` | Search failed: { $detail } | खोज नहीं हो सकी: { $detail } | Messages |  |
| `common-load-failed` | Could not load: { $detail } | लोड नहीं हो सका: { $detail } | Messages |  |
| `messages-conversations-empty` | No conversations yet. | अभी कोई बातचीत नहीं है। | Messages |  |
| `messages-count` | { $count } messages | { $count } संदेश | Messages |  |
| `messages-compose-placeholder` | Write a message | संदेश लिखें | Messages |  |
| `messages-send-button` | Send | भेजें | Messages |  |
| `messages-send-failed` | Send failed: { $detail } | भेजा नहीं जा सका: { $detail } | Messages |  |
| `request-toggle-button` | Send a request | अनुरोध भेजें | Messages |  |
| `request-description-placeholder` | Describe what you need | बताएँ आपको क्या चाहिए | Messages |  |
| `request-categories-placeholder` | Categories (comma-separated, e.g. cycling, repair) | श्रेणियाँ (कॉमा से अलग करें, जैसे साइकिल, मरम्मत) | Messages |  |
| `request-listing-placeholder` | Listing ID (optional) | लिस्टिंग आईडी (ज़रूरी नहीं) | Messages |  |
| `request-submit-button` | Submit request | अनुरोध जमा करें | Messages |  |
| `request-description-required` | Description is required | विवरण देना ज़रूरी है | Messages |  |
| `request-send-failed` | Could not send request: { $detail } | अनुरोध भेजा नहीं जा सका: { $detail } | Messages |  |
| `messages-load-failed` | Could not load messages: { $detail } | संदेश लोड नहीं हो सके: { $detail } | Messages |  |
| `messages-state-pending` | pending | बाकी | Messages |  |
| `messages-state-deposited` | deposited | मेलबॉक्स में रखा गया | Messages |  |
| `messages-state-delivered` | delivered | पहुँच गया | Messages |  |
| `messages-state-failed` | failed | विफल | Messages |  |
| `messages-state-hint-deposited` | The other side is offline. The message waits in their mailbox. | दूसरा पक्ष ऑफ़लाइन है। संदेश उनके मेलबॉक्स में इंतज़ार कर रहा है। | Messages |  |
| `messages-refused-rate-limited` | Not accepted: this person limits messages from new contacts. Try again later. | स्वीकार नहीं किया गया: यह व्यक्ति नए संपर्कों से आने वाले संदेश सीमित रखता है। बाद में फिर कोशिश करें। | Messages |  |
| `messages-refused-other` | Not accepted: { $reason }. | स्वीकार नहीं किया गया: { $reason }। | Messages |  |
| `quote-superseded-notice` | This quote answers an earlier version of your request. | यह कोटेशन आपके अनुरोध के पहले वाले संस्करण का जवाब है। | Messages |  |
| `messages-unfiled-card` | Unfiled card | दर्ज न हुआ कार्ड | Messages |  |
| `messages-message-deleted` | (message deleted) | (संदेश हटा दिया गया) | Messages |  |
| `messages-no-body` | (no body) | (कोई पाठ नहीं) | Messages |  |
| `messages-retry-button` | Retry | फिर भेजें | Messages |  |
| `messages-delete-button` | Delete | हटाएँ | Messages |  |
| `messages-decline-button` | Decline | अस्वीकार करें | Messages |  |
| `messages-decline-confirm` | Confirm Decline | अस्वीकार की पुष्टि करें | Messages |  |
| `messages-decline-failed` | Decline failed: { $detail } | अस्वीकार नहीं हो सका: { $detail } | Messages |  |
| `common-cancel` | Cancel | रद्द करें | Messages |  |
| `quote-scope-placeholder` | Scope of work | काम का ब्योरा | Messages |  |
| `quote-currency-placeholder` | Currency (e.g. EUR, USD) | मुद्रा (जैसे EUR, USD) | Messages |  |
| `quote-amount-placeholder` | Total amount (e.g. 45.00) | कुल राशि (जैसे 45.00) | Messages |  |
| `quote-tax-placeholder` | Tax included (optional, e.g. 5.00) | कर शामिल (ज़रूरी नहीं, जैसे 5.00) | Messages |  |
| `quote-fees-placeholder` | Fees included (optional, e.g. 0.00) | शुल्क शामिल (ज़रूरी नहीं, जैसे 0.00) | Messages |  |
| `quote-payee-placeholder` | Payee name or company | पैसे पाने वाले का नाम या कंपनी | Messages |  |
| `quote-timing-after` | Payment after work | काम के बाद भुगतान | Messages |  |
| `quote-timing-before` | Payment before work | काम से पहले भुगतान | Messages |  |
| `quote-address-placeholder` | Service address | सेवा का पता | Messages |  |
| `quote-cancellation-placeholder` | Cancellation terms | रद्द करने की शर्तें | Messages |  |
| `quote-refund-placeholder` | Refund terms | वापसी की शर्तें | Messages |  |
| `quote-dispute-placeholder` | Dispute path | विवाद सुलझाने का तरीका | Messages |  |
| `quote-hours-placeholder` | Expires in hours | कितने घंटों में खत्म हो | Messages |  |
| `quote-scope-required` | Scope is required | काम का ब्योरा देना ज़रूरी है | Messages |  |
| `quote-currency-required` | Currency is required | मुद्रा देना ज़रूरी है | Messages |  |
| `quote-currency-unknown` | Unknown currency code "{ $currency }" | अज्ञात मुद्रा कोड "{ $currency }" | Messages |  |
| `quote-amount-required` | Amount is required | राशि देना ज़रूरी है | Messages |  |
| `quote-payee-required` | Payee is required | पैसे पाने वाले का नाम देना ज़रूरी है | Messages |  |
| `quote-failed` | Quote failed: { $detail } | कोटेशन नहीं भेजा जा सका: { $detail } | Messages |  |
| `messages-delete-ask-peer` | Also ask them to delete their copy | उनसे भी अपनी प्रति हटाने को कहें | Messages |  |
| `messages-delete-confirm` | Delete this copy | यह प्रति हटाएँ | Messages |  |
| `messages-delete-failed` | Delete failed: { $detail } | हटाया नहीं जा सका: { $detail } | Messages |  |
| `group-login-again` | Log in again | फिर से लॉग इन करें | Groups |  |
| `group-message-deleted` | This message was deleted. | यह संदेश हटा दिया गया था। | Groups |  |
| `group-retry-button` | Retry | फिर भेजें | Groups |  |
| `group-delete-button` | Delete | हटाएँ | Groups |  |
| `group-profile-named` | { $author } named the group “{ $name }” | { $author } ने समूह का नाम “{ $name }” रखा | Groups |  |
| `group-name-placeholder` | Group name | समूह का नाम | Groups |  |
| `group-new-button` | New group | नया समूह | Groups |  |
| `group-name-label` | Group Name | समूह का नाम | Groups |  |
| `group-hidden-summary` | Hidden groups ({ $count }) | छिपाए हुए समूह ({ $count }) | Groups |  |
| `group-list-empty` | No groups yet. | अभी कोई समूह नहीं है। | Groups |  |
| `group-unnamed` | Unnamed group | बिना नाम का समूह | Groups |  |
| `group-message-count` | { $count } messages | { $count } संदेश | Groups |  |
| `group-unhide-button` | Unhide | फिर दिखाएँ | Groups |  |
| `group-load-failed` | Could not load group: { $detail } | समूह लोड नहीं हो सका: { $detail } | Groups |  |
| `group-compose-placeholder` | Write a message... | संदेश लिखें... | Groups |  |
| `group-send-button` | Send | भेजें | Groups |  |
| `group-rename-placeholder` | New name | नया नाम | Groups |  |
| `group-rename-button` | Rename | नाम बदलें | Groups |  |
| `group-owner-line` | Owner: { $owner } | मालिक: { $owner } | Groups |  |
| `group-owner-line-you` | Owner: { $owner } (you) | मालिक: { $owner } (आप) | Groups |  |
| `group-members-heading` | Members ({ $count }) | सदस्य ({ $count }) | Groups |  |
| `group-owner-badge` | (owner) | (मालिक) | Groups |  |
| `group-address-placeholder` | Address or Person DID | पता या व्यक्ति का DID | Groups |  |
| `group-add-member-button` | Add member | सदस्य जोड़ें | Groups |  |
| `group-key-changed` | Group key changed here: { $date } (epoch { $epoch }) | समूह की कुंजी यहाँ बदली: { $date } (चरण { $epoch }) | Groups |  |
| `group-transcript-check` | Transcript check: { $digest } | संदेश-सूची जाँच: { $digest } | Groups |  |
| `group-sync-button` | Sync now | अभी सिंक करें | Groups |  |
| `group-hide-button` | Hide this group | इस समूह को छिपाएँ | Groups |  |
| `membership-issuer-changed-check` | This directory now says it speaks for a different group than the one you first saw. Its answer was not used. | यह डायरेक्टरी अब कहती है कि वह उस समूह से अलग समूह की ओर से बोलती है जो आपने पहले देखा था। उसका जवाब इस्तेमाल नहीं किया गया। | Membership words |  |
| `membership-reason-issuer-changed` | the group this directory speaks for is not the one you first saw when you added it | यह डायरेक्टरी जिस समूह की ओर से बोलती है वह वह नहीं है जो आपने इसे जोड़ते समय पहली बार देखा था | Membership words |  |
| `membership-reason-issuer-not-pinned` | this node has not yet recorded which group this directory speaks for | इस नोड ने अभी दर्ज नहीं किया कि यह डायरेक्टरी किस समूह की ओर से बोलती है | Membership words |  |
| `membership-unknown-no-answer` | Membership: unknown ({ $source } gave no answer this node could use) | सदस्यता: अज्ञात ({ $source } ने ऐसा कोई जवाब नहीं दिया जिसे यह नोड इस्तेमाल कर सके) | Membership words |  |
| `membership-valid` | Member of { $name }, checked on your node. Group withdrawals checked { $age }. | { $name } का सदस्य, आपके नोड पर जाँचा गया। समूह की वापसियाँ जाँची गईं: { $age }। | Membership words |  |
| `membership-none` | No membership shown by { $source } | { $source } ने कोई सदस्यता नहीं दिखाई | Membership words |  |
| `membership-unknown` | Membership: unknown ({ $reason }) | सदस्यता: अज्ञात ({ $reason }) | Membership words |  |
| `membership-refused` | Membership evidence did NOT check out -- treat as unknown | सदस्यता का सबूत जाँच में सही नहीं निकला -- इसे अज्ञात मानें | Membership words |  |
| `membership-expired` | Membership expired on { $date } | सदस्यता { $date } को खत्म हो गई | Membership words |  |
| `membership-out-of-scope` | Membership does not cover this listing: { $outside } | सदस्यता इस लिस्टिंग को शामिल नहीं करती: { $outside } | Membership words |  |
| `membership-refusal-none` | This group has no membership on record for you. | इस समूह के पास आपकी कोई सदस्यता दर्ज नहीं है। | Membership words |  |
| `membership-this-group` | this group | इस समूह | Membership words |  |
| `membership-publish-not-admitted` | this group did not admit this listing. { $why } | इस समूह ने यह लिस्टिंग स्वीकार नहीं की। { $why } | Membership words |  |
| `membership-issuer-unrecorded` | This node has not recorded which group this directory speaks for. | इस नोड ने दर्ज नहीं किया कि यह डायरेक्टरी किस समूह की ओर से बोलती है। | Membership words |  |
| `membership-issuer-pinned` | The group this directory said it is when you added it: { $issuer } | जब आपने इस डायरेक्टरी को जोड़ा, तब इसने जिस समूह की होने की बात कही: { $issuer } | Membership words |  |
| `membership-never-checked` | never checked | कभी जाँची नहीं गई | Membership words |  |
| `membership-checked` | checked { $age } | जाँची गई: { $age } | Membership words |  |
| `membership-revoked` | Membership revoked: the group withdrew it on { $date }.{ $has_reason -> [yes] { " " }Reason given: { $reason } *[no] { "" } } | सदस्यता रद्द: समूह ने इसे { $date } को वापस ले लिया।{ $has_reason -> [yes] { " " }बताया गया कारण: { $reason } *[no] { "" } } | Membership words |  |
| `membership-suspended` |  { $scope -> [listing] This listing is suspended *[membership] Membership suspended } by the group under its rule "{ $rule }", since { $since }, { $until -> [date] until { $date } *[lifted] until the group lifts it }.{ $has_reason -> [yes] { " " }Reason given: { $reason } *[no] { "" } } |  { $scope -> [listing] यह लिस्टिंग *[membership] सदस्यता } समूह ने अपने नियम "{ $rule }" के तहत { $since } से रोकी है, { $until -> [date] { $date } तक *[lifted] जब तक समूह इसे हटा न दे }।{ $has_reason -> [yes] { " " }बताया गया कारण: { $reason } *[no] { "" } } | Membership words |  |
| `memberships-title` | Memberships | सदस्यताएँ | Memberships |  |
| `memberships-intro` | Each line is a listing owner's membership in a group, as this node worked it out from the group's own signed statements. It is the copy you hold, not a live answer. | हर पंक्ति किसी लिस्टिंग के मालिक की किसी समूह में सदस्यता है, जैसा इस नोड ने समूह के अपने हस्ताक्षरित बयानों से निकाला। यह आपके पास रखी प्रति है, ताज़ा जवाब नहीं। | Memberships |  |
| `memberships-load-failed` | Could not load memberships: { $detail } | सदस्यताएँ लोड नहीं हो सकीं: { $detail } | Memberships |  |
| `memberships-empty` | No memberships checked yet. One appears here after a search returns a result from a group the listing owner belongs to. | अभी कोई सदस्यता जाँची नहीं गई। जब खोज में किसी ऐसे समूह का नतीजा आएगा जिसका लिस्टिंग का मालिक सदस्य है, तब एक यहाँ दिखेगी। | Memberships |  |
| `membership-row-source` | Group directory: { $label } | समूह की डायरेक्टरी: { $label } | Memberships |  |
| `membership-row-member` | Listing owner: { $did } | लिस्टिंग का मालिक: { $did } | Memberships |  |
| `membership-last-error` | Last check failed: { $error } | पिछली जाँच नहीं हो सकी: { $error } | Memberships |  |
| `membership-check-again` | Check again | फिर जाँचें | Memberships |  |
| `membership-checked-now` | Checked just now. | अभी जाँचा गया। | Memberships |  |
| `membership-unreachable` | Could not reach this directory; this is the copy you already held. | इस डायरेक्टरी तक नहीं पहुँच सके; यह वही प्रति है जो आपके पास पहले से थी। | Memberships |  |
| `membership-check-failed` | Could not check: { $detail } | जाँच नहीं हो सकी: { $detail } | Memberships |  |
| `directory-check-again` | Check membership again | सदस्यता फिर जाँचें | Directory |  |
| `directory-membership-alone` | Membership on its own, not tied to this listing: { $words } | सदस्यता अपने आप में, इस लिस्टिंग से जुड़े बिना: { $words } | Directory |  |
| `directory-check-failed` | Could not check: { $detail } | जाँच नहीं हो सकी: { $detail } | Directory |  |
| `directory-checked-now` | Checked just now. | अभी जाँचा गया। | Directory |  |
| `directory-unreachable` | Could not reach this directory. Showing what was { $checked }. | इस डायरेक्टरी तक नहीं पहुँच सके। जो पहले से रखा है वह दिखा रहे हैं: { $checked }। | Directory |  |
| `directory-source-error-not-started` | this installation was busy and did not start the call | यह इंस्टॉलेशन व्यस्त था और उसने कॉल शुरू नहीं की | Directory |  |
| `directory-source-error-timed-out` | the directory did not answer in time | डायरेक्टरी ने समय पर जवाब नहीं दिया | Directory |  |
| `directory-source-error-not-found` | no directory answers at that address | उस पते पर कोई डायरेक्टरी जवाब नहीं देती | Directory |  |
| `directory-source-error-refused` | the directory refused the request | डायरेक्टरी ने अनुरोध मना कर दिया | Directory |  |
| `directory-source-error-unreadable` | the directory's answer could not be read | डायरेक्टरी का जवाब पढ़ा नहीं जा सका | Directory |  |
| `directory-title` | Directories | डायरेक्टरी | Directory |  |
| `directory-intro` | A directory is a place you choose to search. It is optional: you can always reach a listing owner directly from a link they send you. Your node checks every result itself -- a directory is never trusted. | डायरेक्टरी वह जगह है जहाँ आप खोजना चुनते हैं। यह ज़रूरी नहीं है: लिस्टिंग के मालिक का भेजा लिंक हो तो आप सीधे उन तक पहुँच सकते हैं। आपका नोड हर नतीजा खुद जाँचता है -- डायरेक्टरी पर कभी भरोसा नहीं किया जाता। | Directory |  |
| `directory-yours-heading` | Your directories | आपकी डायरेक्टरी | Directory |  |
| `directory-load-failed` | Could not load your directories: { $detail } | आपकी डायरेक्टरी लोड नहीं हो सकीं: { $detail } | Directory |  |
| `directory-empty` | No directories added yet. | अभी कोई डायरेक्टरी नहीं जोड़ी गई। | Directory |  |
| `directory-last-error` | last error: { $reason } | पिछली गड़बड़ी: { $reason } | Directory |  |
| `directory-answered-recently` | answered recently | हाल में जवाब दिया | Directory |  |
| `directory-remove-failed` | Remove failed: { $detail } | हटाया नहीं जा सका: { $detail } | Directory |  |
| `directory-add-heading` | Add a directory | डायरेक्टरी जोड़ें | Directory |  |
| `directory-add-help` | Paste the Roym Directory service DID a friend, a referral or a well-known list gave you. There is no directory of directories. | किसी दोस्त, सिफ़ारिश या जानी-मानी सूची से मिला Roym डायरेक्टरी सेवा का DID यहाँ चिपकाएँ। डायरेक्टरियों की कोई डायरेक्टरी नहीं है। | Directory |  |
| `directory-add-did-placeholder` | did:key:... (Roym Directory service DID) | did:key:... (Roym डायरेक्टरी सेवा का DID) | Directory |  |
| `directory-add-label-placeholder` | your label for it (optional) | इसके लिए आपका नाम (ज़रूरी नहीं) | Directory |  |
| `directory-add-button` | Add directory | डायरेक्टरी जोड़ें | Directory |  |
| `directory-added-note` | Added. Note: { $probe }. | जोड़ दी गई। ध्यान दें: { $probe }। | Directory |  |
| `directory-added-silent` | Added, but the directory did not answer a check just now. | जोड़ दी गई, पर डायरेक्टरी ने अभी की जाँच का जवाब नहीं दिया। | Directory |  |
| `directory-added` | Added. | जोड़ दी गई। | Directory |  |
| `directory-add-failed` | Could not add: { $detail } | जोड़ी नहीं जा सकी: { $detail } | Directory |  |
| `directory-search-heading` | Search your directories | अपनी डायरेक्टरी में खोजें | Directory |  |
| `directory-search-text-placeholder` | words to look for (optional) | खोजने के शब्द (ज़रूरी नहीं) | Directory |  |
| `directory-search-categories-placeholder` | categories, comma separated (optional) | श्रेणियाँ, कॉमा से अलग करें (ज़रूरी नहीं) | Directory |  |
| `directory-search-help` | Matching is on whole words in the title, summary and categories. Punctuation is ignored, so searching for 50% finds 50. There is no ranking and no paid placement -- results are shown newest first. | मिलान शीर्षक, सार और श्रेणियों के पूरे शब्दों पर होता है। विराम-चिह्न छोड़ दिए जाते हैं, इसलिए 50% खोजने पर 50 मिलता है। कोई रैंकिंग नहीं है और पैसे देकर ऊपर आने की सुविधा नहीं है -- नतीजे सबसे नए पहले दिखते हैं। | Directory |  |
| `directory-search-button` | Search | खोजें | Directory |  |
| `directory-searching` | Searching... | खोज रहे हैं... | Directory |  |
| `directory-no-sources` | You have not added any directories. Add one above, or reach a listing owner by a direct link. | आपने कोई डायरेक्टरी नहीं जोड़ी है। ऊपर एक जोड़ें, या लिस्टिंग के मालिक तक सीधे लिंक से पहुँचें। | Directory |  |
| `directory-searched` | { $count } directories searched. | { $count } डायरेक्टरी में खोजा गया। | Directory |  |
| `directory-more-than-returned` | this directory had more matches than it would return | इस डायरेक्टरी के पास जितने नतीजे उसने लौटाए उनसे ज़्यादा थे | Directory |  |
| `directory-results-heading` | Results | नतीजे | Directory |  |
| `directory-no-results` | No results. | कोई नतीजा नहीं। | Directory |  |
| `directory-untitled` | (untitled) | (बिना शीर्षक) | Directory |  |
| `directory-hit-categories` | Categories: { $categories } | श्रेणियाँ: { $categories } | Directory |  |
| `directory-hit-offered-by` | Offered by: { $issuer } | देने वाला: { $issuer } | Directory |  |
| `directory-signature-ok` | signature: checked on your node | हस्ताक्षर: आपके नोड पर जाँचा गया | Directory |  |
| `directory-signature-bad` | signature: did NOT check out -- treat as unknown | हस्ताक्षर: जाँच में सही नहीं निकला -- इसे अज्ञात मानें | Directory |  |
| `directory-revocation` | revocation: { $status } | वापसी: { $status } | Directory |  |
| `directory-revocation-good` | good | ठीक | Directory |  |
| `directory-revocation-unknown` | unknown | अज्ञात | Directory |  |
| `directory-seen-at` | Seen at: { $from } | यहाँ दिखा: { $from } | Directory |  |
| `directory-versions-differ` | Two directories disagree about which version of this offer is current. The newer signed version is shown. | दो डायरेक्टरी इस पर सहमत नहीं हैं कि इस पेशकश का कौन-सा संस्करण मौजूदा है। हस्ताक्षर वाला नया संस्करण दिखाया गया है। | Directory |  |
| `directory-engage-button` | Message this listing owner | इस लिस्टिंग के मालिक को संदेश भेजें | Directory |  |
| `directory-engage-gone` | The full listing is no longer in this run. | पूरी लिस्टिंग अब इस खोज में नहीं है। | Directory |  |
| `directory-engage-address` | Conversation address: { $address } -- open the Messages tab and start a conversation with this address. | बातचीत का पता: { $address } -- संदेश टैब खोलें और इस पते से बातचीत शुरू करें। | Directory |  |
| `directory-engage-failed` | Could not open: { $detail } | खोला नहीं जा सका: { $detail } | Directory |  |
| `directory-truncated` | More results were found than are shown here. | यहाँ दिखाए गए नतीजों से ज़्यादा नतीजे मिले थे। | Directory |  |
| `directory-refused-heading` | Refused evidence | अस्वीकार किया गया सबूत | Directory |  |
| `directory-refused-intro` | One or more directories served these, but their signatures did not check out on your node. They are shown so you know a directory served them. They are never treated as real offers. | एक या अधिक डायरेक्टरी ने ये दिए, पर इनके हस्ताक्षर आपके नोड पर सही नहीं निकले। ये इसलिए दिखाए गए हैं कि आपको पता रहे कि किसी डायरेक्टरी ने ये दिए। इन्हें कभी असली पेशकश नहीं माना जाता। | Directory |  |
| `directory-refused-fallback` | signature did not verify | हस्ताक्षर सही नहीं निकला | Directory |  |
| `directory-served-by` | Served by: { $sources } | इनसे मिला: { $sources } | Directory |  |
| `directory-refused-truncated` | More refused evidence was served than is shown here. | यहाँ दिखाए गए अस्वीकृत सबूत से ज़्यादा सबूत दिए गए थे। | Directory |  |
| `synorg-title` | Your SynOrg | आपका SynOrg | SynOrg |  |
| `synorg-intro` | A SynOrg is a directory you run: listing owners publish their own listings to it, and anyone can search it. Creating settings is what turns this installation into one. | SynOrg वह डायरेक्टरी है जो आप चलाते हैं: लिस्टिंग के मालिक अपनी लिस्टिंग इसमें प्रकाशित करते हैं, और कोई भी इसमें खोज सकता है। सेटिंग बनाते ही यह इंस्टॉलेशन SynOrg बन जाता है। | SynOrg |  |
| `synorg-settings-load-failed` | Could not load settings: { $detail } | सेटिंग लोड नहीं हो सकीं: { $detail } | SynOrg |  |
| `synorg-status-running` | This installation runs the SynOrg "{ $name }". | यह इंस्टॉलेशन SynOrg "{ $name }" चलाता है। | SynOrg |  |
| `synorg-status-none` | This installation runs no SynOrg yet. | यह इंस्टॉलेशन अभी कोई SynOrg नहीं चलाता। | SynOrg |  |
| `synorg-settings-edit` | Edit settings | सेटिंग बदलें | SynOrg |  |
| `synorg-settings-create` | Create your SynOrg | अपना SynOrg बनाएँ | SynOrg |  |
| `synorg-name-placeholder` | name | नाम | SynOrg |  |
| `synorg-rules-placeholder` | the rules a person reads before deciding to trust this group | वे नियम जो कोई व्यक्ति इस समूह पर भरोसा करने से पहले पढ़ता है | SynOrg |  |
| `synorg-categories-placeholder` | categories, comma separated | श्रेणियाँ, कॉमा से अलग करें | SynOrg |  |
| `synorg-support-placeholder` | support contact | सहायता के लिए संपर्क | SynOrg |  |
| `synorg-dispute-placeholder` | how a dispute is handled | विवाद कैसे सुलझाया जाता है | SynOrg |  |
| `synorg-retention-placeholder` | how many days a publication is kept | प्रकाशन कितने दिन रखा जाता है | SynOrg |  |
| `synorg-save-button` | Save settings | सेटिंग सहेजें | SynOrg |  |
| `synorg-retention-invalid` | Retention must be a whole number of days. | रखने की अवधि दिनों की पूर्ण संख्या होनी चाहिए। | SynOrg |  |
| `synorg-label-name` | Name | नाम | SynOrg |  |
| `synorg-label-rules` | Rules | नियम | SynOrg |  |
| `synorg-label-categories` | Categories | श्रेणियाँ | SynOrg |  |
| `synorg-label-support` | Support contact | सहायता के लिए संपर्क | SynOrg |  |
| `synorg-label-dispute` | Dispute path | विवाद सुलझाने का तरीका | SynOrg |  |
| `synorg-label-retention` | Retention (days) | रखने की अवधि (दिन) | SynOrg |  |
| `synorg-limit-heading` | Publication limit | प्रकाशन की सीमा | SynOrg |  |
| `synorg-limit-intro` | How many listings one listing owner may publish here inside one window. Taking a listing down never counts against this. | एक समय-सीमा के अंदर लिस्टिंग का एक मालिक यहाँ कितनी लिस्टिंग प्रकाशित कर सकता है। लिस्टिंग हटाना इस सीमा में कभी नहीं गिना जाता। | SynOrg |  |
| `synorg-limit-window-placeholder` | window (seconds) | समय-सीमा (सेकंड में) | SynOrg |  |
| `synorg-limit-max-placeholder` | max publications per window | हर समय-सीमा में प्रकाशनों की अधिकतम संख्या | SynOrg |  |
| `synorg-limit-save` | Save limit | सीमा सहेजें | SynOrg |  |
| `synorg-limit-whole` | Both values must be whole numbers. | दोनों मान पूर्ण संख्याएँ होने चाहिए। | SynOrg |  |
| `synorg-roster-heading` | Members | सदस्य | SynOrg |  |
| `synorg-roster-note` | The roster is local. It is never served over the wire. | यह सदस्य-सूची केवल यहीं रहती है। इसे नेटवर्क पर कभी नहीं भेजा जाता। | SynOrg |  |
| `synorg-member-did-placeholder` | member DID | सदस्य का DID | SynOrg |  |
| `synorg-roster-note-placeholder` | note (optional) | नोट (ज़रूरी नहीं) | SynOrg |  |
| `synorg-roster-add` | Add member | सदस्य जोड़ें | SynOrg |  |
| `synorg-roster-not-removed` | Not removed: { $detail } | हटाया नहीं गया: { $detail } | SynOrg |  |
| `synorg-roster-load-failed` | Could not load members: { $detail } | सदस्य लोड नहीं हो सके: { $detail } | SynOrg |  |
| `synorg-roster-add-failed` | Add failed: { $detail } | जोड़ा नहीं जा सका: { $detail } | SynOrg |  |
| `synorg-published-heading` | Published here | यहाँ प्रकाशित | SynOrg |  |
| `synorg-published-empty` | Nothing published here yet. | यहाँ अभी कुछ प्रकाशित नहीं हुआ। | SynOrg |  |
| `synorg-published-by` | by { $issuer } | { $issuer } द्वारा | SynOrg |  |
| `synorg-unpublished` | removed from future search | आगे की खोज से हटा दिया गया | SynOrg |  |
| `synorg-publications-load-failed` | Could not load publications: { $detail } | प्रकाशन लोड नहीं हो सके: { $detail } | SynOrg |  |
| `synorg-members-heading` | Memberships and decisions | सदस्यताएँ और फ़ैसले | SynOrg |  |
| `synorg-members-load-failed` | Could not load memberships: { $detail } | सदस्यताएँ लोड नहीं हो सकीं: { $detail } | SynOrg |  |
| `synorg-issue-heading` | Issue a membership credential | सदस्यता का प्रमाणपत्र जारी करें | SynOrg |  |
| `synorg-issue-help` | A listing owner gives you their DID outside this app. The credential you sign here is what lets them publish to this group, and what a searcher's node checks. | लिस्टिंग का मालिक अपना DID आपको इस ऐप के बाहर देता है। आप यहाँ जो प्रमाणपत्र पर हस्ताक्षर करते हैं, उसी से वे इस समूह में प्रकाशित कर पाते हैं, और खोजने वाले का नोड उसी को जाँचता है। | SynOrg |  |
| `synorg-issue-button` | Issue credential | प्रमाणपत्र जारी करें | SynOrg |  |
| `synorg-issue-enter-did` | Enter the member's DID. | सदस्य का DID लिखें। | SynOrg |  |
| `synorg-issue-choose-category` | Choose at least one category. | कम से कम एक श्रेणी चुनें। | SynOrg |  |
| `synorg-issue-days-range` | Days must be a whole number from 1 to 730. | दिन 1 से 730 के बीच की पूर्ण संख्या होने चाहिए। | SynOrg |  |
| `synorg-issued` | Issued. | जारी हो गया। | SynOrg |  |
| `synorg-issue-failed` | Not issued: { $detail } | जारी नहीं हुआ: { $detail } | SynOrg |  |
| `synorg-label-member-did` | Member DID | सदस्य का DID | SynOrg |  |
| `synorg-covers-label` | Covers: | इनके लिए: | SynOrg |  |
| `synorg-label-valid-days` | Valid for (days) | मान्यता (दिन) | SynOrg |  |
| `synorg-suspend-heading` | Suspend a member | किसी सदस्य को रोकें | SynOrg |  |
| `synorg-suspend-listing-placeholder` | listing id (leave empty to suspend the whole membership) | लिस्टिंग आईडी (पूरी सदस्यता रोकनी हो तो खाली छोड़ें) | SynOrg |  |
| `synorg-suspend-rule-placeholder` | which of your rules | आपका कौन-सा नियम | SynOrg |  |
| `synorg-reason-placeholder` | reason | कारण | SynOrg |  |
| `synorg-suspend-days-placeholder` | days (empty = until you lift it) | दिन (खाली = जब तक आप न हटाएँ) | SynOrg |  |
| `synorg-suspend-button` | Suspend | रोकें | SynOrg |  |
| `synorg-suspend-required` | A member DID and the rule are required. | सदस्य का DID और नियम देना ज़रूरी है। | SynOrg |  |
| `synorg-suspend-days-min` | Days must be a whole number of at least 1. | दिन कम से कम 1 की पूर्ण संख्या होने चाहिए। | SynOrg |  |
| `synorg-suspended` | Suspended. | रोक दिया गया। | SynOrg |  |
| `synorg-suspend-failed` | Not suspended: { $detail } | रोका नहीं गया: { $detail } | SynOrg |  |
| `synorg-label-one-listing` | One listing only | केवल एक लिस्टिंग | SynOrg |  |
| `synorg-label-rule` | Rule | नियम | SynOrg |  |
| `synorg-label-reason` | Reason | कारण | SynOrg |  |
| `synorg-label-how-long` | For how long | कितने समय के लिए | SynOrg |  |
| `synorg-credentials-heading` | Credentials you have issued | आपके जारी किए हुए प्रमाणपत्र | SynOrg |  |
| `synorg-credentials-empty` | None issued yet. | अभी कोई जारी नहीं हुआ। | SynOrg |  |
| `synorg-credential-covers` | covers { $categories } | इनके लिए: { $categories } | SynOrg |  |
| `synorg-credential-status-revoked` | revoked | रद्द | SynOrg |  |
| `synorg-credential-status-current` | current | चालू | SynOrg |  |
| `synorg-credential-status-replaced` | replaced | बदला हुआ | SynOrg |  |
| `synorg-revoke-button` | Revoke | रद्द करें | SynOrg |  |
| `synorg-revoke-failed` | Not revoked: { $detail } | रद्द नहीं हुआ: { $detail } | SynOrg |  |
| `synorg-decisions-heading` | Suspensions and lifts | रोक और रोक हटाना | SynOrg |  |
| `synorg-decisions-empty` | No decisions yet. | अभी कोई फ़ैसला नहीं है। | SynOrg |  |
| `synorg-decision-lifted` | lifted a suspension | रोक हटाई | SynOrg |  |
| `synorg-decision-suspended-membership` | suspended the whole membership | पूरी सदस्यता रोकी | SynOrg |  |
| `synorg-decision-suspended-listing` | suspended one listing ({ $id }) | एक लिस्टिंग रोकी ({ $id }) | SynOrg |  |
| `synorg-decision-rule` | rule: { $rule } | नियम: { $rule } | SynOrg |  |
| `synorg-lift-button` | Lift | हटाएँ | SynOrg |  |
| `synorg-lift-failed` | Not lifted: { $detail } | हटाया नहीं गया: { $detail } | SynOrg |  |
