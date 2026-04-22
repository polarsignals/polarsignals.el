/nvfuwlvlpl/ {
    x=2
}

match($0, /^( *)/, a) {
    if (x == 0){
        print $0
    } else {
        if (x == 1) {
            print a[0] "\"" ENVIRON["VERSION_NAME"] "\""
        }
        x=x-1
    }
}
