#!/bin/bash
# module setup for 99emergency-failure

install() {
    inst_hook emergency 99 "${moddir}/failure.sh"
}
