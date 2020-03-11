# CoreOS installation on IBM Z

IBM s390x machines usually have DASD (Direct Access Storage Device) disks.

To use DASD as a Linux hard disk we have to perform several steps:

1. Low-level format it using `dasdfmt` tool
2. Create partitions on it using `fdasd` tool
3. Copy each partition from CoreOS image to corresponding partition on DASD
4. Install boot loader using `zipl` tool
5. Mark DASD as next boot device using `chreipl` tool
