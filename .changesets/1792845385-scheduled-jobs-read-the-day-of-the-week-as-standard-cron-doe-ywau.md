---
type: patch
---

Scheduled jobs read the day of the week as standard cron does: 0 and 7 are Sunday, 1 is Monday, and 1-5 is Monday to Friday. A job saved with a numeric day of the week now fires on the day it names, one day later than before (it ran a day early), and an expression using 0 for Sunday is no longer refused
