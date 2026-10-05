Small logs for the tests, one per format the dashboard reads.

`csv_receiver.txt` is the first 160 lines of a bench capture from the old CSV receiver, as the
serial monitor saved it: boards 2 and 7 indoors, board 7 restarting 43 times. No row has a position.
`flash_dump.txt` is the first records of three boots from one flash dump (40, 40 and 2), with the
packet counts in its source lines changed to match. No record has a position.
`receiver.txt` is made up, in the raw receiver's format: two receivers, two boards, heartbeats, a
radio error and a short packet.
